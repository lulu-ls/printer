#!/usr/bin/env node

/**
 * SumatraPDF 打包脚本
 *
 * 从项目根 bin/ 目录复制对应架构的 SumatraPDF 可执行文件到
 * src-tauri/binaries/，使用 Tauri sidecar 的命名约定：
 *   sumatrapdf-<target-triple>.exe
 *
 * 用法：
 *   node scripts/bundle-sumatrapdf.mjs              → 复制当前主机架构的 binary
 *   node scripts/bundle-sumatrapdf.mjs --target <t> → 复制指定架构
 *   node scripts/bundle-sumatrapdf.mjs --build [t]  → 复制 + 运行 cargo tauri build
 *   node scripts/bundle-sumatrapdf.mjs --all        → 复制所有三个架构（准备批量构建）
 *
 * 架构映射：
 *   x86_64-pc-windows-msvc  ←  bin/SumatraPDF-3.6.1-64.exe
 *   i686-pc-windows-msvc    ←  bin/SumatraPDF-3.6.1-32.exe
 *   aarch64-pc-windows-msvc ←  bin/SumatraPDF-3.6.1-arm64.exe
 */

import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, '..');
const SRC_TAURI = join(ROOT, 'src-tauri');
const BINARIES_DIR = join(SRC_TAURI, 'binaries');
const BIN_DIR = join(ROOT, 'bin');

// 架构 → SumatraPDF 文件名后缀映射
const ARCH_TO_SUFFIX = {
  'x86_64-pc-windows-msvc': '64',
  'i686-pc-windows-msvc': '32',
  'aarch64-pc-windows-msvc': 'arm64',
};

// Node.js process.arch → Rust target triple
const HOST_ARCH_MAP = {
  x64: 'x86_64-pc-windows-msvc',
  ia32: 'i686-pc-windows-msvc',
  arm64: 'aarch64-pc-windows-msvc',
};

function getHostTriple() {
  const triple = HOST_ARCH_MAP[process.arch];
  if (!triple) {
    console.error(`不支持的 Java 架构: ${process.arch}`);
    process.exit(1);
  }
  return triple;
}

/**
 * 为指定的 target triple 复制 SumatraPDF binary。
 */
function deploy(targetTriple) {
  const suffix = ARCH_TO_SUFFIX[targetTriple];
  if (!suffix) {
    console.error(`不支持的 target triple: ${targetTriple}`);
    console.error(`支持的: ${Object.keys(ARCH_TO_SUFFIX).join(', ')}`);
    process.exit(1);
  }

  const srcName = `SumatraPDF-3.6.1-${suffix}.exe`;
  const src = join(BIN_DIR, srcName);
  const dstName = `sumatrapdf-${targetTriple}.exe`;
  const dst = join(BINARIES_DIR, dstName);

  if (!existsSync(src)) {
    console.error(`[ERROR] 未找到源文件: ${src}`);
    console.error(`请确保 bin/ 目录下有 SumatraPDF-3.6.1-${suffix}.exe`);
    process.exit(1);
  }

  // 确保目标目录存在
  if (!existsSync(BINARIES_DIR)) {
    mkdirSync(BINARIES_DIR, { recursive: true });
  }

  copyFileSync(src, dst);
  console.log(`[OK] ${srcName} → ${dstName}`);
}

/**
 * 运行 cargo tauri build。
 */
function runBuild(targetTriple) {
  const args = ['cargo', 'tauri', 'build'];
  if (targetTriple) {
    args.push('--target', targetTriple);
  }
  console.log(`\n[BUILD] ${args.join(' ')}`);
  execSync(args.join(' '), {
    cwd: SRC_TAURI,
    stdio: 'inherit',
  });
}

// ── 入口 ─────────────────────────────────

const args = process.argv.slice(2);
const modeAll = args.includes('--all');
const modeBuild = args.includes('--build');

// SumatraPDF 仅用于 Windows 打印。macOS/Linux 构建无需部署 sidecar，
// 且 Tauri 在非 Windows 平台不会打包 externalBin（tauri.windows.conf.json 限定）。
// 未显式指定目标（--all/--build/--target=）时，非 Windows 平台直接跳过。
const hasExplicitTarget = modeAll || modeBuild || args.some(a => a.startsWith('--target='));
if (!hasExplicitTarget && process.platform !== 'win32') {
  console.log('[SKIP] 非 Windows 平台，跳过 SumatraPDF sidecar 部署');
  process.exit(0);
}

// 收集要部署的目标
let targets = [];

if (modeAll) {
  targets = Object.keys(ARCH_TO_SUFFIX);
} else if (modeBuild) {
  // --build 后可指定目标 triple
  const buildTarget = args.find(a => a.startsWith('--target='))?.split('=')[1]
    || args.find((a, i) => a === '--build' && args[i + 1] && !args[i + 1].startsWith('-')) // --build <triple>
    || getHostTriple();
  targets = [buildTarget];
} else if (args.includes('--target=')) {
  targets = [args.find(a => a.startsWith('--target=')).split('=')[1]];
} else {
  targets = [getHostTriple()];
}

// 部署
for (const t of targets) {
  deploy(t);
}

// 构建
if (modeBuild) {
  // 如果有多个 target 指定 --build，每个都构建
  for (const t of targets) {
    runBuild(t);
  }
}
