// 双语词典：zh（简体中文，默认）/ en（English）
export const STRINGS = {
  zh: {
    unsupportedType: '该文件类型不支持，仅支持 PDF、Office 文档、图片、文本文件',
    unsupportedSkipped: '已跳过 {n} 个不支持的文件',
    appTitle: '打印机助手',
    emptyTitle: '拖拽文件到此处，或点击选择',
    emptySubtitle: '支持 PNG、JPG、JPEG、PDF 等常见格式',
    addFiles: '添加更多文件',
    printFilter: '可打印文件',
    clearList: '清空列表',
    noPrinter: '暂未检测到打印机',
    noPrinterDesc: '点击此区域前往系统设置添加打印机',
    noPrinterOpenSettings: '未检测到打印机，已打开「打印机与扫描仪」设置，添加后重新点击打印',
    printerPrefix: '打印机：',
    defaultPrinter: '系统默认打印机',
    printBtn: '开始打印',
    printing: '打印中...',
    dropToAdd: '释放以添加文件',
    loTitle: '需要 LibreOffice',
    loDownload: '下载 LibreOffice',
    loDownloadUrl: 'https://zh-cn.libreoffice.org/download/libreoffice/',
    loSkip: '跳过这些文件',
    cancel: '取消',
    failTag: '打印失败',
    queuedTag: '队列中',
    removeTitle: '移除',
    selectError: '文件选择出错：',
    noOtherPrinter: '没有其他打印机可选',
    selected: '已选择：',
    fileList: '文件列表（{n}）',
    loPrompt:
      '检测到 {n} 个文件需要 LibreOffice 才能打印（Office 文档等）。可下载安装后重试，或跳过这些文件、只打印其余内容。',
    loOpened: '已打开 LibreOffice 下载页，安装后重新点击打印',
    pleaseSelect: '请先选择文件',
    skippedAll: '已跳过全部文件，没有可打印的内容',
    skippedN: '已跳过 {n} 个需要 LibreOffice 的文件',
    exported: '已生成 PDF 到临时文件夹，并在访达中打开',
    sentN: '已发送 {n} 个文件到 {printer}',
    resultOkFail: '成功 {ok}，失败 {fail}（失败项已保留在列表，可重试）',
  },
  en: {
    unsupportedType: 'This file type is not supported. Only PDF, Office documents, images, and text files are supported.',
    unsupportedSkipped: 'Skipped {n} unsupported file(s)',
    appTitle: 'Printer Assistant',
    emptyTitle: 'Drag files here, or click to select',
    emptySubtitle: 'Supports PNG, JPG, JPEG, PDF and other common formats',
    addFiles: 'Add more files',
    printFilter: 'Printable files',
    clearList: 'Clear list',
    noPrinter: 'No printer detected',
    noPrinterDesc: 'Tap here to open system settings and add a printer',
    noPrinterOpenSettings: 'No printer detected. Printer settings opened — add one and click print again',
    printerPrefix: 'Printer: ',
    defaultPrinter: 'system default printer',
    printBtn: 'Print',
    printing: 'Printing...',
    dropToAdd: 'Drop to add files',
    loTitle: 'LibreOffice Required',
    loDownload: 'Download LibreOffice',
    loDownloadUrl: 'https://www.libreoffice.org/download/',
    loSkip: 'Skip these files',
    cancel: 'Cancel',
    failTag: 'Print failed',
    queuedTag: 'Queued',
    removeTitle: 'Remove',
    selectError: 'File selection error: ',
    noOtherPrinter: 'No other printer to choose',
    selected: 'Selected: ',
    fileList: 'File list ({n})',
    loPrompt:
      '{n} file(s) require LibreOffice to print (Office documents, etc.). Download and install it to retry, or skip these files and print the rest.',
    loOpened: 'LibreOffice download page opened. Click print again after installing.',
    pleaseSelect: 'Please select files first',
    skippedAll: 'All files skipped, nothing to print',
    skippedN: 'Skipped {n} file(s) that require LibreOffice',
    exported: 'PDFs exported to the temp folder and opened in Finder',
    sentN: 'Sent {n} file(s) to {printer}',
    resultOkFail: 'Success {ok}, failed {fail} (failed items kept in the list, can retry)',
  },
};

export function t(key, params) {
  const lang = window.__lang || 'zh';
  let s = STRINGS[lang]?.[key] ?? STRINGS.zh[key] ?? key;
  if (params) {
    for (const k in params) {
      s = s.split('{' + k + '}').join(params[k]);
    }
  }
  return s;
}
