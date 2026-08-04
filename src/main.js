import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { t } from './i18n.js';

// 打印按钮图标（语言切换时需要重建 innerHTML）
const PRINT_SVG = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="20" height="20">
    <polyline points="6 9 6 2 18 2 18 9"/>
    <path d="M6 18H4a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-2"/>
    <rect x="6" y="14" width="12" height="8"/>
  </svg>`;

// ── 状态 ────────────────────────────────────────────
const state = {
  files: [],           // { id, path, name, size, ext, status, error }
  printerName: '',
  printers: [],
  noPrinter: false,
  printerOnline: null,   // true=在线绿 / false=离线红 / null=未知灰（由后台轮询写入）
  printerStatuses: {},   // 各打印机在线状态记录：name -> true/false/null（后台轮询更新）
  lang: 'zh',          // zh | en
  printing: false,     // 正在打印中，禁止操作文件
  cancelPrinting: false, // 用户点击"取消打印"标记，打印循环检测后停止
  currentJobId: null,  // 当前已提交的 CUPS 任务号，用于取消
  settings: {
    copies: 1,
    color: true,       // true=彩色 / false=黑白
    duplex: 'off',     // 'off' | 'long' | 'short'
    orientation: 'portrait', // 'portrait' | 'landscape'
  },
};

// Demo 模式（环境变量 DEMO=true）：伪造打印机 + 模拟打印成功，便于录演示视频
let isDemo = false;

// 文件自增 id（保证卡片增量渲染时的稳定标识）与卡片 DOM 映射
let fileIdSeq = 0;
const cardEls = new Map();

// ── DOM 引用 ────────────────────────────────────────
const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => document.querySelectorAll(sel);

const emptyState   = $('#emptyState');
const emptyDrop    = $('#emptyDropZone');
const filePage     = $('#filePage');
const dropZone     = $('#dropZone');
const fileSection  = $('#fileSection');
const fileCards    = $('#fileCards');
const fileListTitle = $('#fileListTitle');
const clearBtn     = $('#clearBtn');
const bottomBar    = $('#bottomBar');
const printerCard  = $('#printerCard');
const printerTitle = $('#printerTitle');
const printerDesc  = $('#printerDesc');
const printerCaret = $('#printerCaret');
const printerDot   = $('#printerDot');
const printerPopup = $('#printerPopup');
const printerPopupList = $('#printerPopupList');
const printBtn     = $('#printBtn');
const printBtnWrap = $('#printBtnWrap');
const printBtnTooltip = $('#printBtnTooltip');
const dragOverlay  = $('#dragOverlay');
const loModal      = $('#loModal');
const loModalMsg   = $('#loModalMsg');
const loDownloadBtn = $('#loDownloadBtn');
const loSkipBtn    = $('#loSkipBtn');
const loCancelBtn  = $('#loCancelBtn');
const titlebar     = $('#titlebar');
const settingsBtn  = $('#settingsBtn');
const settingsPopup = $('#settingsPopup');
const settingsCopies = $('#settingsCopies');
const splitBtn = document.querySelector('.split-btn');

// 标题栏拖动用（Overlay 样式下原生 data-tauri-drag-region 行为不稳定，
// 这里显式调用系统 API 保证整条标题栏都可拖动，包含标题文字区域）
titlebar.addEventListener('mousedown', (e) => {
  // 左键才触发拖动，避免与右键菜单等冲突
  if (e.button !== 0) return;
  getCurrentWindow().startDragging();
});

// 全局前端错误 -> 写入 Rust 日志（与后端同一份日志文件，便于统一排查）
function logFrontend(level, msg) {
  invoke('log_message', { level, msg }).catch(() => {});
}
window.addEventListener('error', (e) => {
  const msg = `${e.message} @ ${e.filename || ''}:${e.lineno || ''}:${e.colno || ''}`;
  logFrontend('error', msg);
});
window.addEventListener('unhandledrejection', (e) => {
  const r = e.reason;
  const detail = r && (r.stack || r.message) ? (r.stack || r.message) : String(r);
  logFrontend('error', `Unhandled rejection: ${detail}`);
});

// 文件列表事件委托：删除按钮 + 文件名悬停浮框（仅当被截断时显示完整名）
let nameTipEl = null;
function ensureNameTip() {
  if (!nameTipEl) {
    nameTipEl = document.createElement('div');
    nameTipEl.className = 'name-tooltip';
    nameTipEl.style.display = 'none';
    document.body.appendChild(nameTipEl);
  }
  return nameTipEl;
}
fileCards.addEventListener('click', (e) => {
  // 失败文件的重试按钮：任何时候都可点（打印中也行）
  const retryBtn = e.target.closest('.file-retry-btn');
  if (retryBtn) {
    const f = state.files.find(x => x.id === Number(retryBtn.dataset.id));
    if (f && f.status === 'fail') {
      retryFile(f);
    }
    return;
  }

  // 删除按钮：打印中只允许删除"排队中/转换中/失败"的文件（不能删正在打印的）
  const btn = e.target.closest('.file-delete-btn');
  if (btn) {
    const id = Number(btn.dataset.id);
    if (state.printing) {
      const f = state.files.find(x => x.id === id);
      if (!f || f.status === 'printing' || f.status === 'sent') {
        toast(t('printingLocked'));
        return;
      }
    }
    removeFileById(id);
  }
});
fileCards.addEventListener('mouseover', (e) => {
  const nameEl = e.target.closest('.file-name');
  if (nameEl && nameEl.scrollWidth > nameEl.clientWidth + 1) {
    const tip = ensureNameTip();
    tip.textContent = nameEl.textContent;
    tip.style.display = 'block';
    const rect = nameEl.getBoundingClientRect();
    const tw = tip.offsetWidth;
    let left = rect.left + rect.width / 2 - tw / 2;
    left = Math.max(8, Math.min(left, window.innerWidth - tw - 8));
    tip.style.left = left + 'px';
    tip.style.top = (rect.top - tip.offsetHeight - 8) + 'px';
  }
  // 失败标签 -> 显示错误原因
  const failTag = e.target.closest('.file-status-tag.fail');
  if (failTag) {
    const err = failTag.dataset.error;
    if (err) {
      const tip = ensureNameTip();
      tip.textContent = err;
      tip.style.display = 'block';
      const rect = failTag.getBoundingClientRect();
      const tw = tip.offsetWidth;
      let left = rect.left + rect.width / 2 - tw / 2;
      left = Math.max(8, Math.min(left, window.innerWidth - tw - 8));
      tip.style.left = left + 'px';
      tip.style.top = (rect.top - tip.offsetHeight - 8) + 'px';
    }
  }
});
fileCards.addEventListener('mouseout', (e) => {
  if (e.target.closest('.file-name') && nameTipEl) nameTipEl.style.display = 'none';
  if (e.target.closest('.file-status-tag.fail') && nameTipEl) nameTipEl.style.display = 'none';
});

// ── 初始化 ──────────────────────────────────────────
async function init() {
  console.log('[DEBUG] init() 开始');
  logFrontend('info', 'init() 开始');

  // 平台相关：Windows 使用原生标题栏，隐藏自绘的深色标题栏
  try {
    const pf = await invoke('platform');
    if (pf === 'windows') {
      titlebar.style.display = 'none';
    }
  } catch (_) { /* 忽略 */ }

  // Demo 模式（环境变量 DEMO=true，无需真实打印机即可录屏）
  try { isDemo = await invoke('is_demo'); } catch (_) { isDemo = false; }

  // 检测打印机
  try {
    if (isDemo) {
      state.printers = ['Demo Printer (HP LaserJet)'];
      state.printerName = 'Demo Printer (HP LaserJet)';
    } else {
      state.printers = await invoke('list_printers');
      const def = await invoke('get_default_printer');
      state.printerName = def || '';
    }
  } catch (e) {
    state.printers = [];
    state.printerName = '';
  }
  state.noPrinter = state.printers.length === 0;
  updatePrinterUI();
  refreshStatuses();  // 启动时查一次打印机状态
  showEmpty();

  // 拖拽事件
  logFrontend('info', 'setupDragDrop...');
  setupDragDrop();

  // 空状态点击
  logFrontend('info', `emptyDrop=${!!emptyDrop}, dropZone=${!!dropZone}`);
  emptyDrop.addEventListener('click', () => {
    logFrontend('info', 'emptyDrop clicked');
    selectFiles();
  });
  dropZone.addEventListener('click', () => {
    logFrontend('info', 'dropZone clicked');
    selectFiles();
  });

  logFrontend('info', 'init() 注册事件监听器...');

  // 清空按钮
  clearBtn.addEventListener('click', () => { logFrontend('info', 'clearAll 点击'); clearAll(); });

  // 打印机卡片点击
  printerCard.addEventListener('click', togglePrinterPopup);

  // 打印按钮：空闲时开始打印；打印中时切换为"取消打印"
  printBtn.addEventListener('click', () => {
    if (state.printing) {
      cancelPrinting();
    } else {
      startPrint();
    }
  });

  // ── 打印设置面板 ──────────────────────────────
  settingsBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    const show = settingsPopup.style.display !== 'block';
    settingsPopup.style.display = show ? 'block' : 'none';
  });
  document.addEventListener('click', (e) => {
    if (!settingsPopup.contains(e.target) && !e.target.closest('#settingsBtn')) {
      settingsPopup.style.display = 'none';
    }
  });
  settingsCopies.addEventListener('change', () => {
    let v = parseInt(settingsCopies.value, 10);
    if (!v || v < 1) v = 1;
    if (v > 99) v = 99;
    settingsCopies.value = v;
    state.settings.copies = v;
  });

  // 分段选择器：颜色 / 双面 / 方向
  function bindSeg(segId, apply) {
    const seg = document.getElementById(segId);
    seg.addEventListener('click', (e) => {
      const btn = e.target.closest('.seg-item');
      if (!btn) return;
      seg.querySelectorAll('.seg-item').forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      apply(btn.dataset.value);
    });
  }
  bindSeg('settingsColorSeg', (v) => { state.settings.color = v !== 'gray'; });
  bindSeg('settingsDuplexSeg', (v) => { state.settings.duplex = v; });
  bindSeg('settingsOrientationSeg', (v) => { state.settings.orientation = v; });

  // 右下角标：跳转 GitHub 项目
  const badge = document.getElementById('footerBadge');
  logFrontend('info', `badge 元素=${!!badge}`);
  badge.addEventListener('click', (e) => {
    logFrontend('info', 'badge 点击');
    e.preventDefault();
    invoke('open_url', { url: 'https://github.com/lulu-ls/printer' }).catch(() => {});
  });

  // 语言变更时更新 footer badge 提示
  // （主 listen 在下方 applyLang 处）

  // dropzone 折叠：用 hysteresis（迟滞）避免临界抖动
  // 一旦折叠，只有回到顶部附近才展开；展开后只有超过阈值才折叠
  let dropzoneRafId = null;
  let dropzoneCollapsed = false;
  fileSection.addEventListener('scroll', () => {
    if (dropzoneRafId) return;
    dropzoneRafId = requestAnimationFrame(() => {
      dropzoneRafId = null;
      const canScroll = fileSection.scrollHeight > fileSection.clientHeight;
      if (!canScroll) {
        dropZone.classList.remove('compact');
        dropzoneCollapsed = false;
        return;
      }
      if (dropzoneCollapsed) {
        // 已折叠：只有回到顶部才展开
        if (fileSection.scrollTop <= 5) {
          dropZone.classList.remove('compact');
          dropzoneCollapsed = false;
        }
      } else {
        // 未折叠：超过阈值才折叠
        if (fileSection.scrollTop > 40) {
          dropZone.classList.add('compact');
          dropzoneCollapsed = true;
        }
      }
    });
  }, { passive: true });

  // 点击空白关闭弹出层
  document.addEventListener('click', (e) => {
    if (!printerPopup.contains(e.target) && !printerCard.contains(e.target)) {
      closePrinterPopup();
    }
  });

  // LibreOffice 提示弹窗按钮
  loDownloadBtn.addEventListener('click', async () => {
    try {
      await invoke('open_url', { url: t('loDownloadUrl') });
    } catch (_) { /* 忽略 */ }
    closeLoModal('download');
  });
  loSkipBtn.addEventListener('click', () => closeLoModal('skip'));
  loCancelBtn.addEventListener('click', () => closeLoModal('cancel'));

  // 监听下载进度事件（不再自动下载，保留为空以防遗留事件）
  // 清理旧的 progress 监听器（如果存在）

  // 语言：启动时读取偏好，并监听原生菜单的语言切换事件
  try {
    state.lang = await invoke('get_language');
  } catch (_) {
    state.lang = 'zh';
  }
  applyLang(state.lang);
  // 初始化 badge 提示（必须在 applyLang 之后）
  document.getElementById('footerBadge').dataset.tip = t('footerTip');
  listen('language-changed', (e) => {
    state.lang = e.payload;
    applyLang(state.lang);
    document.getElementById('footerBadge').dataset.tip = t('footerTip');
  });

  // 每 3 秒自动检测一次打印机在线状态（即使不操作也会刷新绿/红点）
  setInterval(() => {
    if (!state.noPrinter) refreshStatuses();
  }, 3000);

  // ── 打印进度事件（后端在转换/发送阶段推送） ──────────
  listen('print-progress', (e) => {
    const { fileId, status } = e.payload || {};
    if (fileId == null) return;
    const f = state.files.find(x => x.id === fileId);
    if (!f) return;
    if (status === 'converting') {
      f.status = 'converting';
      const card = cardEls.get(f.id);
      if (card) updateFileCard(card, f);
    } else if (status === 'sending') {
      f.status = 'printing';
      const card = cardEls.get(f.id);
      if (card) updateFileCard(card, f);
    }
  });

  // 启动时清理 1 天前的临时转换文件
  invoke('clean_temp_files', { olderThanDays: 1 }).catch(() => {});
}

// ── 国际化：根据语言刷新全部文案（含动态文本） ───────────
function applyLang(lang) {
  state.lang = lang;
  window.__lang = lang;
  document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
  document.title = t('appTitle');

  // 静态文案（带 data-i18n 的元素）
  document.querySelectorAll('[data-i18n]').forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });

  // 动态文案
  updatePrinterUI();
  fileListTitle.textContent = t('fileList', { n: state.files.length });
  if (state.files.length) renderFiles();
}

// ── 文件选择 ────────────────────────────────────────
async function selectFiles() {
  try {
    const selected = await open({
      multiple: true,
      filters: [{
        name: t('printFilter'),
        extensions: [
          'pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx',
          'jpg', 'jpeg', 'png', 'gif', 'bmp', 'tiff', 'tif', 'webp',
          'txt', 'rtf', 'csv', 'md', 'html', 'htm'
        ]
      }]
    });
    if (!selected) return;

    const paths = Array.isArray(selected) ? selected : [selected];
    const supported = ['pdf','doc','docx','xls','xlsx','ppt','pptx',
      'jpg','jpeg','png','gif','bmp','tiff','tif','webp','txt','rtf','csv','md','html','htm'];
    let added = 0, skipped = 0;
    for (const p of paths) {
      if (p && typeof p === 'string') {
        const ext = (p.split('.').pop() || '').toLowerCase();
        if (supported.includes(ext)) {
          await addFile(p);
          added++;
        } else {
          skipped++;
        }
      }
    }
    renderFiles();
    if (added === 0 && skipped > 0) {
      toast(t('unsupportedType'));
    } else if (skipped > 0) {
      toast(t('unsupportedSkipped', { n: skipped }));
    }
  } catch (e) {
    toast(t('selectError') + e);
  }
}

async function addFile(path) {
  if (state.printing) return;
  // 去重
  if (state.files.some(f => f.path === path)) return;
  try {
    const info = await invoke('get_file_info', { path });
    info.id = ++fileIdSeq;
    info.status = 'pending'; // pending | ok | fail
    info.error = '';
    state.files.push(info);
  } catch (e) {
    // 忽略不支持的文件
  }
}

function removeFileById(id) {
  const idx = state.files.findIndex((f) => f.id === id);
  if (idx >= 0) state.files.splice(idx, 1);
  renderFiles();
}

// 重试单个失败文件（不进入主打印队列，独立调用后端打印）
async function retryFile(f) {
  if (state.printing && f.status === 'printing') return;
  f.status = 'converting';
  f.error = '';
  const card = cardEls.get(f.id);
  if (card) {
    card.classList.remove('file-card--next');
    updateFileCard(card, f);
  }
  try {
    const jobId = await invoke('print_file', {
      path: f.path,
      printerName: state.printerName,
      settings: buildPrintSettings(),
      fileId: f.id,
    });
    if (jobId && jobId !== 'ok') state.currentJobId = jobId;
    // 成功：移除文件
    const idx = state.files.indexOf(f);
    if (idx >= 0) state.files.splice(idx, 1);
    const cardToRemove = cardEls.get(f.id);
    if (cardToRemove && cardToRemove.isConnected) {
      cardToRemove.classList.add('file-card--exit');
      if (isCardVisible(cardToRemove)) burstParticles(cardToRemove);
      const onDone = () => {
        cardToRemove.removeEventListener('animationend', onDone);
        collapseCard(f.id, cardToRemove);
      };
      cardToRemove.addEventListener('animationend', onDone, { once: true });
      setTimeout(() => { if (cardToRemove.isConnected) collapseCard(f.id, cardToRemove); }, 800);
    }
    if (state.files.length === 0) showEmpty();
  } catch (e) {
    f.status = 'fail';
    f.error = String(e && e.message ? e.message : e);
    const failCard = cardEls.get(f.id);
    if (failCard) updateFileCard(failCard, f);
    toast(f.error);
  }
}

let clearing = false;
function clearAll() {
  if (state.printing || clearing) return;
  if (state.files.length === 0) return;

  // 先播放所有卡片的退场动画，动画结束（约500ms）后再清空，
  // 避免 showEmpty 提前隐藏文件页把动画截断
  clearing = true;
  for (const card of cardEls.values()) {
    if (!card.isConnected) continue;
    if (isCardVisible(card)) {
      card.animate(
        [{ opacity: 1 }, { opacity: 0 }],
        { duration: 750, easing: 'ease', fill: 'forwards' }
      );
      // 延迟一帧再爆粒子，避免压过淡出动画首帧
      requestAnimationFrame(() => burstParticles(card));
    } else {
      card.classList.add('file-card--exit');
    }
  }

  setTimeout(() => {
    clearing = false;
    state.files = [];
    // 直接移除全部卡片，避免 renderFiles 对已退场卡片重复播动画
    for (const card of cardEls.values()) {
      card.remove();
    }
    cardEls.clear();
    renderFiles();
  }, 800);
}

// 读取当前设置面板，组装传给后端的 settings 参数
function buildPrintSettings() {
  return {
    copies: state.settings.copies,
    color: state.settings.color,
    duplex: state.settings.duplex === 'off' ? false : true,
    landscape: state.settings.orientation === 'landscape',
  };
}

// ── 渲染文件列表 ────────────────────────────────────
function renderFiles() {
  fileListTitle.textContent = t('fileList', { n: state.files.length });

  if (state.files.length > 0) showFileList();
  else showEmpty();

  // 增量渲染：仅新文件播进场动画，已有文件仅更新状态（不重播）
  // 倒序遍历，让最新添加的文件显示在列表最上方
  const present = new Set();
  for (let i = state.files.length - 1; i >= 0; i--) {
    const f = state.files[i];
    present.add(f.id);
    let card = cardEls.get(f.id);
    if (!card) {
      card = createFileCard(f);
      cardEls.set(f.id, card);
      card.classList.add('file-card--enter');
      card.addEventListener('animationend', () => card.classList.remove('file-card--enter'), { once: true });
      fileCards.prepend(card); // 新卡片插入到最前面（倒序显示）
    } else {
      updateFileCard(card, f);
      // 老卡片原位不动，避免打乱 DOM 顺序导致被删卡片跳到顶部
    }
  }
  // 已移除的文件：仅对可视区的卡片播动画（不可见的不做无用功）
  for (const [id, card] of cardEls) {
    if (!present.has(id)) {
      if (isCardVisible(card)) {
        burstParticles(card, 720);
        card.classList.add('file-card--exit');
        card.addEventListener('animationend', () => collapseCard(id, card), { once: true });
        setTimeout(() => collapseCard(id, card), 500);
      } else {
        // 不在可视区内直接清理，不浪费动画性能
        card.remove();
        cardEls.delete(id);
      }
    }
  }

  updateDropZoneAccept();
  updatePrintBtnDisabledTip();
}

// 状态标签 HTML（失败/打印中/转换中/排队）
function statusTagHTML(f) {
  if (f.status === 'fail') {
    return `<span class="file-status-tag fail" data-error="${escHtml(f.error)}">${t('failTag')}</span>`
      + `<button class="file-retry-btn" data-id="${f.id}" type="button" title="${t('retry')}">`
      + `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="13" height="13">`
      + `<path d="M23 4v6h-6"/><path d="M1 20v-6h6"/>`
      + `<path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>`
      + `</svg></button>`;
  } else if (f.status === 'printing') {
    return `<span class="file-status-tag printing"><span class="spinner"></span>${t('printing')}</span>`;
  } else if (f.status === 'converting') {
    return `<span class="file-status-tag converting"><span class="spinner"></span>${t('converting')}</span>`;
  } else if (f.status === 'queued') {
    return `<span class="file-status-tag queued">${t('queuedTag')}</span>`;
  }
  return '';
}

// 文件卡片 HTML 模板（create 用）
function cardTemplate(f) {
  return `
    <div class="file-type-badge">${escHtml(f.ext)}</div>
    <div class="file-info">
      <span class="file-name">${escHtml(f.name)}</span>
      <span class="file-size">${escHtml(f.size)}</span>
    </div>
    <div class="file-status-area">${statusTagHTML(f)}</div>
    <button class="file-delete-btn" data-id="${f.id}" type="button" title="${t('removeTitle')}">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="3 6 5 6 21 6"/>
        <path d="M19 6l-1 14H6L5 6"/>
        <path d="M10 11v6M14 11v6"/>
        <path d="M9 6V4h6v2"/>
      </svg>
    </button>`;
}

function fileCardClass(f) {
  return 'file-card'
    + (f.status === 'fail' ? ' failed' : '')
    + (f.status === 'printing' ? ' printing' : '')
    + (f.status === 'converting' ? ' converting' : '')
    + (f.status === 'queued' ? ' queued' : '');
}

function createFileCard(f) {
  const el = document.createElement('div');
  el.className = fileCardClass(f);
  el.innerHTML = cardTemplate(f);
  return el;
}

function updateFileCard(card, f) {
  card.className = fileCardClass(f);
  // 只更新状态区域，不重建整个卡片（避免 innerHTML 解析开销导致主线程卡顿）
  const area = card.querySelector('.file-status-area');
  if (area) area.innerHTML = statusTagHTML(f);
}

// ── 卡片删除粒子爆散（Telegram 碎屑风格） ──────────────
function burstParticles(card, count = 140) {
  const rect = card.getBoundingClientRect();
  const colors = ['#ff5f57','#febc2e','#28c840','#4b6bfb','#ff6b6b','#ffd93d','#6bcb77','#4d96ff','#e040fb'];

  for (let i = 0; i < count; i++) {
    const p = document.createElement('div');
    p.className = 'particle';
    // 在卡片矩形区域内随机取起始位置
    const px = rect.left + Math.random() * rect.width;
    const py = rect.top + Math.random() * rect.height;
    const angle = Math.random() * 360;
    const dist = 30 + Math.random() * 100;
    const rad = angle * (Math.PI / 180);
    const tx = Math.cos(rad) * dist;
    const ty = Math.sin(rad) * dist;
    const size = 1.5 + Math.random() * 5.5; // 细碎粒子
    p.style.left = (px - size / 2) + 'px';
    p.style.top = (py - size / 2) + 'px';
    p.style.width = size + 'px';
    p.style.height = size + 'px';
    p.style.borderRadius = Math.random() > 0.5 ? '50%' : '2px';
    p.style.background = colors[Math.floor(Math.random() * colors.length)];
    document.body.appendChild(p);

    // 从随机起始位置向随机方向飞散、旋转、缩小
    p.animate([
      { opacity: 1, transform: 'translate(0, 0) scale(1)' },
      { opacity: 0, transform: `translate(${tx}px, ${ty}px) rotate(${angle * 2}deg) scale(0.15)` },
    ], { duration: 750, easing: 'ease-out', fill: 'forwards' });

    setTimeout(() => { if (p.parentNode) p.remove(); }, 1000);
  }
}

// ── 卡片移除后塌缩空间，让下方文件平滑上移 ──────────
function collapseCard(id, card) {
  if (!card.isConnected || cardEls.get(id) !== card) return;

  // 收集下方卡片，并记录它们此刻的屏幕位置（卡片尚未移除）
  const downCards = [];
  let sib = card.nextElementSibling;
  while (sib && sib.classList.contains('file-card')) {
    downCards.push(sib);
    sib = sib.nextElementSibling;
  }
  const oldTops = downCards.map(c => c.getBoundingClientRect().top);

  // 直接把卡片从布局中移除（下方卡片会瞬间上跳）
  card.style.display = 'none';

  // 用 getBoundingClientRect 测出下方卡片上跳后的新位置，计算差值
  downCards.forEach((c, i) => {
    const newTop = c.getBoundingClientRect().top;
    const diff = oldTops[i] - newTop; // 差值 = 卡片高度 + 间隙
    c.style.transform = `translateY(${diff}px)`;
  });

  // 强制重排使 transform 生效
  downCards.forEach(c => void c.offsetHeight);

  // 下一帧启动过渡：将 transform 归零 → 下方卡片从原位平滑上移
  requestAnimationFrame(() => {
    downCards.forEach(c => {
      c.style.transition = 'transform 225ms ease-out';
      c.style.transform = 'translateY(0)';
    });
  });

  // 清理（守卫防止重复执行）
  let cleaned = false;
  const clean = () => {
    if (cleaned) return;
    cleaned = true;
    downCards.forEach(c => { c.style.transition = ''; c.style.transform = ''; });
    card.remove();
    cardEls.delete(id);
  };
  if (downCards.length === 0) {
    clean();
    return;
  }
  downCards.forEach(c => c.addEventListener('transitionend', clean, { once: true }));
  setTimeout(clean, 500);
}

// ── 可见性检测：只有可视区内的卡片才播删除动画 ─────
function isCardVisible(card) {
  const content = document.getElementById('content');
  const cr = content.getBoundingClientRect();
  const cardR = card.getBoundingClientRect();
  return cardR.bottom > cr.top && cardR.top < cr.bottom;
}

// ── UI 状态切换（带过渡动画） ───────────────────────
function showEmpty() {
  emptyState.style.display = 'flex';

  // 从文件列表退回空状态：空状态从上方滑入居中（与 showFileList 相反）
  if (filePage.style.display !== 'none' && filePage.style.display !== '') {
    emptyState.classList.add('is-hiding'); // 起始：上方 -36px、透明
    filePage.classList.remove('is-showing');
    setTimeout(() => {
      filePage.style.display = 'none';
      // 触发空状态向下滑入并淡出 → 居中
      void emptyState.offsetWidth;
      emptyState.classList.remove('is-hiding');
    }, 300);
  } else {
    // 首次启动直接显示（无动画）
    filePage.style.display = 'none';
    emptyState.classList.remove('is-hiding');
  }
  bottomBar.style.display = 'flex';
  printBtn.disabled = true;
  updatePrintBtnDisabledTip();
}

function showFileList() {
  // 空状态整体上移淡出
  emptyState.classList.add('is-hiding');
  setTimeout(() => { emptyState.style.display = 'none'; }, 300);
  // 文件列表页从下方淡入
  filePage.style.display = 'flex';
  void filePage.offsetWidth; // 强制重排以触发过渡
  filePage.classList.add('is-showing');
  bottomBar.style.display = 'flex';
  printBtn.disabled = state.noPrinter;
  updatePrintBtnDisabledTip();
}

// 防止轮询与弹窗刷新重叠（离线打印机 OpenPrinter 可能较慢）
let statusRefreshing = false;

// ── 打印机在线检测（仅由后台定时轮询调用，点击不触发） ──
// 拉取所有打印机最新状态，记录到 state.printerStatuses，并刷新底部圆点；
// 若弹窗开着，也同步列表圆点（仍是后台行为，非点击触发）。
async function refreshStatuses() {
  if (statusRefreshing) return;
  // Demo 模式不检测真实打印机（伪造的一直在线）
  if (isDemo) { statusRefreshing = false; return; }
  statusRefreshing = true;
  try {
    if (state.noPrinter) {
      state.printerOnline = false;
      updatePrinterUI();
      return;
    }
    let all;
    try {
      all = await invoke('printers_status');
    } catch (_) {
      // 查询失败：保留已记录的状态，不盲目覆盖
      return;
    }
    // 记录所有打印机的最新状态（后台持续更新）
    const map = {};
    for (const p of all) map[p.name] = p.online;
    state.printerStatuses = map;
    // 底部当前打印机圆点
    state.printerOnline = map[state.printerName] ?? null;
    updatePrinterUI();

    // 弹窗开着时同步刷新列表圆点（后台行为）
    if (printerPopup.style.display !== 'block') return;
    printerPopupList.querySelectorAll('.printer-popup-item').forEach(item => {
      const name = item.dataset.printer;
      const dot = item.querySelector('.printer-popup-dot');
      if (!dot) return;
      const online = map[name];
      dot.className = 'printer-popup-dot' + (online === true ? ' online' : online === false ? ' offline' : '');
    });
  } finally {
    statusRefreshing = false;
  }
}

// ── 打印机 UI ───────────────────────────────────────
function updatePrinterUI() {
  if (state.noPrinter) {
    printerTitle.textContent = t('noPrinter');
    printerDesc.textContent = t('noPrinterDesc');
    printerCaret.style.display = 'none';
    printerDot.style.display = 'none';
    printerCard.classList.add('no-printer');
    printerCard.title = '';
  } else {
    printerTitle.textContent = t('printerPrefix');
    printerDesc.textContent = state.printerName;
    printerDesc.title = state.printerName;
    printerCaret.style.display = 'block';
    printerCard.classList.remove('no-printer');
    printerDot.style.display = '';
    printerDot.className = 'printer-status-dot'
      + (state.printerOnline === true ? ' online' : state.printerOnline === false ? ' offline' : '');
  }
  updatePrintBtnDisabledTip();
}

function togglePrinterPopup() {
  if (state.noPrinter) {
    openSystemPrinterSettings();
    return;
  }
  if (printerPopup.style.display === 'block') {
    closePrinterPopup();
    return;
  }
  const printers = state.printers;
  if (printers.length <= 1) {
    toast(t('noOtherPrinter'));
    return;
  }

  // 直接展示后台已记录的状态（点击不触发刷新）
  printerPopupList.innerHTML = printers.map(p => {
    const online = state.printerStatuses[p];
    const dotCls = online === true ? ' online' : online === false ? ' offline' : '';
    return `
    <div class="printer-popup-item${p === state.printerName ? ' active' : ''}" data-printer="${escHtml(p)}">
      <span class="printer-popup-dot${dotCls}"></span>
      ${escHtml(p)}
    </div>`;
  }).join('');
  printerPopup.style.display = 'block';
  printerCard.classList.add('open');

  printerPopupList.querySelectorAll('.printer-popup-item').forEach(item => {
    item.addEventListener('click', () => {
      const name = item.dataset.printer;
      state.printerName = name;
      // 仅切换到已记录的状态展示，不触发刷新
      state.printerOnline = state.printerStatuses[name] ?? null;
      closePrinterPopup();              // 纯同步，无异步
      updatePrinterUI();
      toast(t('selected') + name);
    });
  });
  // 注意：不在这里调用 refreshStatuses，状态完全由后台定时轮询负责
}

function closePrinterPopup() {
  printerPopup.style.display = 'none';
  printerCard.classList.remove('open');
}

// ── 打印按钮禁用提示 ───────────────────────────────
// 在打印按钮禁用时，鼠标悬浮显示不能打印的原因。
function updatePrintBtnDisabledTip() {
  let tip = '';
  if (state.noPrinter) {
    tip = t('noPrinter');
  } else if (state.files.length === 0) {
    tip = t('pleaseSelect');
  } else if (state.printing) {
    tip = t('printing');
  }
  printBtnTooltip.textContent = tip;
}

// 鼠标悬浮打印按钮时显示 tooltip
printBtn.addEventListener('mouseenter', () => {
  if (printBtn.disabled && printBtnTooltip.textContent) {
    printBtnTooltip.classList.add('show');
  }
});
printBtn.addEventListener('mouseleave', () => {
  printBtnTooltip.classList.remove('show');
});

// ── 打开系统「打印机与扫描仪」设置 ──────────────────
function openSystemPrinterSettings() {
  try {
    invoke('open_url', { url: 'x-apple.systempreferences:com.apple.Print-Scan-Settings.extension' });
  } catch (_) {
    try { invoke('open_url', { url: 'x-apple.systempreferences:' }); } catch (_) { /* 忽略 */ }
  }
  toast(t('noPrinterOpenSettings'));
}

// ── LibreOffice 提示弹窗 ─────────────────────────────
let loResolve = null;

function showLibreOfficePrompt(count) {
  loModalMsg.textContent = t('loPrompt', { n: count });
  loModal.style.display = 'flex';
  return new Promise((resolve) => { loResolve = resolve; });
}

function closeLoModal(result) {
  loModal.style.display = 'none';
  if (loResolve) {
    loResolve(result);
    loResolve = null;
  }
}

// ── 重置打印状态（取消/跳过时恢复 UI） ────────────
function resetPrintingUI() {
  state.printing = false;
  state.cancelPrinting = false;
  state.currentJobId = null;
  printBtn.disabled = false;
  printBtn.innerHTML = `${PRINT_SVG}<span data-i18n="printBtn">${t('printBtn')}</span>`;
  if (splitBtn) splitBtn.classList.remove('cancel-mode');
  clearBtn.disabled = false;
  fileCards.classList.remove('printing');
  // 恢复文件状态
  for (const f of state.files) {
    if (f.status === 'queued' || f.status === 'printing' || f.status === 'converting') f.status = 'waiting';
    const card = cardEls.get(f.id);
    if (card) updateFileCard(card, f);
  }
}

// ── 取消打印：标记取消，打印循环检测后停止 ──────────
function cancelPrinting() {
  if (!state.printing || state.cancelPrinting) return;
  state.cancelPrinting = true;
  printBtn.disabled = true;
  printBtn.textContent = t('cancelling');
  toast(t('cancelling'));
  logFrontend('info', '用户请求取消打印');
  // 取消当前已提交的任务
  if (state.currentJobId) {
    invoke('cancel_print_job', { jobId: state.currentJobId }).catch(() => {});
    state.currentJobId = null;
  }
}

// 调用后端打印，记录任务号；若已请求取消则立即取消刚提交的任务
async function handlePrintResult(f) {
  const jobId = await invoke('print_file', {
    path: f.path,
    printerName: state.printerName,
    settings: buildPrintSettings(),
    fileId: f.id,
  });
  if (jobId && jobId !== 'ok') {
    state.currentJobId = jobId;
    if (state.cancelPrinting) {
      invoke('cancel_print_job', { jobId }).catch(() => {});
      state.currentJobId = null;
    }
  }
  return jobId;
}

// ── 打印 ────────────────────────────────────────────
async function startPrint() {
  if (state.printing || clearing) {
    logFrontend('info', 'startPrint 跳过：已在打印中/正在清空');
    return;
  }
  try {
    await doPrint();
  } catch (e) {
    logFrontend('error', `startPrint 异常: ${e}`);
    resetPrintingUI();
  }
}

async function doPrint() {
  if (state.files.length === 0) {
    toast(t('pleaseSelect'));
    return;
  }

  // 无打印机：直接打开系统「打印机与扫描仪」设置
  if (state.noPrinter) {
    openSystemPrinterSettings();
    return;
  }

  // ── 立即更新 UI：加载阶段按钮不可点击 ──
  state.printing = true;
  state.cancelPrinting = false;
  printBtn.disabled = true;
  printBtn.innerHTML = `<span class="btn-spinner"></span>${t('printing')}`;
  clearBtn.disabled = true;
  fileCards.classList.add('printing');
  for (const f of state.files) {
    f.status = 'queued';
    const card = cardEls.get(f.id);
    if (card) updateFileCard(card, f);
  }
  // 第一个排队文件加呼吸灯边框
  const firstQ = state.files.find(f => f.status === 'queued');
  if (firstQ) {
    const firstCard = cardEls.get(firstQ.id);
    if (firstCard) firstCard.classList.add('file-card--next');
  }
  // 让浏览器渲染 UI（至少等一帧，避免连续点击时状态更新被合并）
  await new Promise(r => setTimeout(r, 50));

  // Demo 模式：生成 PDF（不实际发送到打印机），供检查输出效果
  if (isDemo) {
    const [loAvailable, officeAvailable] = await Promise.all([
      invoke('libreoffice_available'),
      invoke('office_automation_available')
    ]);
    logFrontend('info', `[demo] LO可用=${loAvailable}, Office可用=${officeAvailable}`);
    const loResults = await Promise.all(state.files.map(f => invoke('needs_libreoffice', { path: f.path })));
    const needLo = state.files.filter((_, i) => loResults[i]);
    if (needLo.length > 0 && !loAvailable && !officeAvailable) {
      const action = await showLibreOfficePrompt(needLo.length);
      if (action === 'cancel') { resetPrintingUI(); return; }
      if (action === 'download') { resetPrintingUI(); toast(t('loOpened')); return; }
      // skip：从队列移除这些文件
      for (const f of needLo) {
        f.status = 'initial';
        const card = cardEls.get(f.id);
        if (card) {
          card.classList.remove('file-card--next');
          updateFileCard(card, f);
        }
      }
      state.files = state.files.filter((f) => !needLo.includes(f));
      if (state.files.length === 0) { resetPrintingUI(); toast(t('skippedAll')); return; }
      toast(t('skippedN', { n: needLo.length }));
      // 给剩余第一个文件加呼吸灯
      const firstRemaining = state.files.find(f => f.status === 'queued');
      if (firstRemaining) {
        const c = cardEls.get(firstRemaining.id);
        if (c) c.classList.add('file-card--next');
      }
    }

    // 进入实际打印阶段：按钮变为可点击的"取消打印"（红色警告样式）
    printBtn.disabled = false;
    printBtn.innerHTML = t('cancelPrint');
    if (splitBtn) splitBtn.classList.add('cancel-mode');

    let ok = 0;
    // 拍快照避免 splice 跳项
    for (const f of [...state.files]) {
      // 取消打印：跳过剩余文件
      if (state.cancelPrinting) {
        if (f.status === 'queued') {
          f.status = 'waiting';
          const c = cardEls.get(f.id);
          if (c) { c.classList.remove('file-card--next'); updateFileCard(c, f); }
        }
        continue;
      }
      const realFile = state.files.find(r => r.id === f.id);
      if (!realFile) continue;

      // 切到"转换中"
      realFile.status = 'converting';
      const card = cardEls.get(realFile.id);
      if (card) {
        card.classList.remove('file-card--next');
        updateFileCard(card, realFile);
      }
      // 给下一个排队中的文件加转圈边框
      const nextQ = state.files.find(f => f.status === 'queued');
      if (nextQ) {
        const nextCard = cardEls.get(nextQ.id);
        if (nextCard) nextCard.classList.add('file-card--next');
      }

      try {
        const out = await invoke('build_pdf', { path: realFile.path });
        console.log('demo pdf:', out);
        ok++;
        // 成功：移除文件
        const idx = state.files.indexOf(realFile);
        if (idx >= 0) state.files.splice(idx, 1);
        const cardToRemove = cardEls.get(realFile.id);
        if (cardToRemove && cardToRemove.isConnected) {
          cardToRemove.classList.add('file-card--exit');
          if (isCardVisible(cardToRemove)) burstParticles(cardToRemove);
          const onDone = () => {
            cardToRemove.removeEventListener('animationend', onDone);
            collapseCard(realFile.id, cardToRemove);
          };
          cardToRemove.addEventListener('animationend', onDone, { once: true });
          setTimeout(() => { if (cardToRemove.isConnected) collapseCard(realFile.id, cardToRemove); }, 800);
        }
      } catch (_) {
        realFile.status = 'fail';
        const failCard = cardEls.get(realFile.id);
        if (failCard) updateFileCard(failCard, realFile);
      }
      await sleep(900);
    }
    state.printing = false;
    state.cancelPrinting = false;
    clearBtn.disabled = false;
    fileCards.classList.remove('printing');
    if (splitBtn) splitBtn.classList.remove('cancel-mode');
    if (state.files.length === 0) showEmpty();
    printBtn.disabled = false;
    printBtn.innerHTML = `${PRINT_SVG}<span data-i18n="printBtn">${t('printBtn')}</span>`;
    toast(t('resultOkFail', { ok, fail: state.files.length }));
    return;
  }

  // 检查可用的转换方案：LO > AppleScript（MS Office）> 提示下载
  const [loAvailable, officeAvailable] = await Promise.all([
    invoke('libreoffice_available'),
    invoke('office_automation_available')
  ]);
  logFrontend('info', `LO可用=${loAvailable}, Office可用=${officeAvailable}`);

  // 并行检查所有文件是否需要 LO（避免顺序 IPC 阻塞）
  const loResults = await Promise.all(state.files.map(f => invoke('needs_libreoffice', { path: f.path })));
  const needLo = state.files.filter((_, i) => loResults[i]);
  logFrontend('info', `需要转换的文件数: ${needLo.length}/${state.files.length}`);

  // LO 和 Office 都不可用时 → 提示下载 LO
  let filesToPrint = state.files;
  if (needLo.length > 0 && !loAvailable && !officeAvailable) {
    const action = await showLibreOfficePrompt(needLo.length);
    if (action === 'cancel') { resetPrintingUI(); return; }
    if (action === 'download') { resetPrintingUI(); toast(t('loOpened')); return; }
    // skip：移除被跳过文件的状态标记
    for (const f of needLo) {
      f.status = 'initial';
      const card = cardEls.get(f.id);
      if (card) updateFileCard(card, f);
    }
    filesToPrint = state.files.filter((f) => !needLo.includes(f));
    if (filesToPrint.length === 0) { resetPrintingUI(); toast(t('skippedAll')); return; }
    toast(t('skippedN', { n: needLo.length }));
  }
  // 如果有 LO 或 Office 其中一种，直接开始打印（LO 走转换管线，Office 走 AppleScript）

  // 进入实际打印阶段：按钮变为可点击的"取消打印"（红色警告样式）
  printBtn.disabled = false;
  printBtn.innerHTML = t('cancelPrint');
  if (splitBtn) splitBtn.classList.add('cancel-mode');

  // 拍快照：避免迭代中 splice 导致跳项
  let ok = 0, fail = 0;
  for (const f of [...filesToPrint]) {
    // 取消打印：跳过剩余文件
    if (state.cancelPrinting) {
      if (f.status === 'queued') {
        f.status = 'waiting';
        const c = cardEls.get(f.id);
        if (c) { c.classList.remove('file-card--next'); updateFileCard(c, f); }
      }
      continue;
    }
    // 文件可能被外部删除（比如在 LO 提示后），跳过
    const realFile = state.files.find(r => r.id === f.id);
    if (!realFile) continue;

    // 切到"转换中"（后端发 sending 事件后变为"打印中"）
    realFile.status = 'converting';
    const card = cardEls.get(realFile.id);
    if (card) updateFileCard(card, realFile);
    // 给下一个排队中的文件加转圈边框
    const nextQ = state.files.find(f => f.status === 'queued');
    if (nextQ) {
      const nextCard = cardEls.get(nextQ.id);
      if (nextCard) nextCard.classList.add('file-card--next');
    }

    try {
      await handlePrintResult(realFile);

      // 用户请求取消：刚提交的任务已取消，当前文件恢复等待
      if (state.cancelPrinting) {
        realFile.status = 'waiting';
        const c = cardEls.get(realFile.id);
        if (c) { c.classList.remove('file-card--next'); updateFileCard(c, realFile); }
        continue;
      }

      // 打印成功：爆散粒子 + 退场动画 + 塌缩
      ok++;
      const idx = state.files.indexOf(realFile);
      if (idx >= 0) state.files.splice(idx, 1);

      const cardToRemove = cardEls.get(realFile.id);
      if (cardToRemove && cardToRemove.isConnected) {
        cardToRemove.classList.add('file-card--exit');
        if (isCardVisible(cardToRemove)) burstParticles(cardToRemove, 180);
        const onDone = () => {
          cardToRemove.removeEventListener('animationend', onDone);
          collapseCard(realFile.id, cardToRemove);
        };
        cardToRemove.addEventListener('animationend', onDone, { once: true });
        setTimeout(() => {
          if (cardToRemove.isConnected) collapseCard(realFile.id, cardToRemove);
        }, 600);
      }
    } catch (e) {
      // 打印失败：标记失败，保留在列表
      fail++;
      realFile.status = 'fail';
      const errMsg = String(e && e.message ? e.message : e);
      realFile.error = errMsg;
      logFrontend('error', `打印失败 [${realFile.name}]: ${errMsg}`);

      // 标记是否需要 LO 提示（AppleScript 失败 + LO 未安装时）
      if (errMsg.toLowerCase().includes('libreoffice')) {
        logFrontend('info', `文件 ${realFile.name} 因缺失 LO 打印失败`);
        realFile._needsLo = true;
      }

      const failCard = cardEls.get(realFile.id);
      if (failCard) updateFileCard(failCard, realFile);
    }

    await sleep(900);
  }

  // ── 打印结束后：AppleScript 失败且 LO 未安装时，弹窗提示下载 ──
  if (!loAvailable && officeAvailable && fail > 0 && state.files.some(f => f._needsLo)) {
    const loAvail = await invoke('libreoffice_available');
    if (!loAvail) {
      const loFailed = state.files.filter(f => f._needsLo);
      const action = await showLibreOfficePrompt(loFailed.length);
      if (action === 'cancel') {
        // 保持原样
      } else if (action === 'download') {
        toast(t('loOpened'));
      } else if (action === 'skip') {
        // 移除需要 LO 的文件
        state.files = state.files.filter(f => !f._needsLo);
        renderFiles();
      }
    }
  }

  // ── 结束，恢复 UI ──────────────────────────────────
  state.printing = false;
  state.cancelPrinting = false;
  clearBtn.disabled = false;
  fileCards.classList.remove('printing');
  if (splitBtn) splitBtn.classList.remove('cancel-mode');
  if (state.files.length === 0) {
    showEmpty();
  }
  // 不调用 renderFiles（collapseCard 已处理 DOM）

  const msg = fail === 0
    ? t('sentN', { n: ok, printer: state.printerName || t('defaultPrinter') })
    : t('resultOkFail', { ok, fail });
  toast(msg);

  printBtn.disabled = false;
  printBtn.innerHTML = `${PRINT_SVG}<span data-i18n="printBtn">${t('printBtn')}</span>`;
}

// ── 拖拽 ────────────────────────────────────────────
function setupDragDrop() {
  const appWindow = getCurrentWindow();

  appWindow.onDragDropEvent((event) => {
    const { type, paths } = event.payload;

    if (type === 'enter' || type === 'over') {
      dragOverlay.style.display = 'flex';
    } else if (type === 'drop') {
      dragOverlay.style.display = 'none';
      if (paths && paths.length) {
        const supported = ['pdf','doc','docx','xls','xlsx','ppt','pptx',
          'jpg','jpeg','png','gif','bmp','tiff','tif','webp','txt','rtf','csv','md','html','htm'];
        let added = 0, unsupported = 0;
        (async () => {
          for (const p of paths) {
            const ext = (p.split('.').pop() || '').toLowerCase();
            if (supported.includes(ext)) {
              await addFile(p);
              added++;
            } else {
              unsupported++;
            }
          }
          renderFiles();
          if (added === 0 && unsupported > 0) {
            toast(t('unsupportedType'));
          } else if (unsupported > 0) {
            toast(t('unsupportedSkipped', { n: unsupported }));
          }
        })();
      }
    } else {
      dragOverlay.style.display = 'none';
    }
  });

  // HTML5 dragover/highlight for drop zones
  [emptyDrop, dropZone].forEach(zone => {
    zone.addEventListener('dragover', (e) => {
      e.preventDefault();
      zone.classList.add('drag-over');
    });
    zone.addEventListener('dragleave', () => {
      zone.classList.remove('drag-over');
    });
  });
}

// 拖拽区接受的文件类型（暂无特殊处理，Tauri 处理）
function updateDropZoneAccept() {}

// ── 工具 ────────────────────────────────────────────
function escHtml(s) {
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

function sleep(ms) {
  return new Promise(r => setTimeout(r, ms));
}

function toast(msg) {
  const el = document.createElement('div');
  el.className = 'toast';
  el.textContent = msg;
  document.body.appendChild(el);
  setTimeout(() => el.remove(), 3000);
}

// ── 启动 ────────────────────────────────────────────
init();
