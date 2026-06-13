// ============================================================
// imsViewer — 前端主逻辑
// Tauri v2 + Canvas + 三栏布局
// ============================================================

import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';

// ===== State =====
const state = {
  folderList: [],          // FolderEntry[]
  currentFileIdx: -1,
  imsInfo: null,           // ImsFileInfo | null
  subImages: [],           // SubImagePayload[]
  currentSubIdx: 0,
  scale: 1.0,
  offsetX: 0,
  offsetY: 0,
  imgObj: null,
  isFitMode: true,          // 适应窗口模式
};

// ===== DOM 引用 =====
const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => document.querySelectorAll(sel);

const dom = {
  canvas: $('#image-canvas'),
  ctx: null,
  thumbnailList: $('#thumbnail-list'),
  thumbnailHeader: $('#thumbnail-header'),
  folderPosition: $('#folder-position'),
  dropHint: $('#drop-hint'),
  infoFileName: $('#info-file-name'),
  infoFileType: $('#info-file-type'),
  infoFileSize: $('#info-file-size'),
  infoImsVersion: $('#info-ims-version'),
  infoImageCount: $('#info-image-count'),
  infoCoverFile: $('#info-cover-file'),
  infoImageName: $('#info-image-name'),
  infoImageFormat: $('#info-image-format'),
  infoImageResolution: $('#info-image-resolution'),
  infoImageSize: $('#info-image-size'),
  infoImageOffset: $('#info-image-offset'),
  infoImageHash: $('#info-image-hash'),
  statusFilePos: $('#status-file-pos'),
  statusSubPos: $('#status-sub-pos'),
  statusZoom: $('#status-zoom'),
};

// ===== 初始化 =====
function init() {
  dom.ctx = dom.canvas.getContext('2d');
  setupCanvasSize();
  setupKeyboard();
  setupMouse();
  setupNativeDrop();    // Tauri 原生拖放
  setupMenuActions();
  setupFolderBar();
  window.addEventListener('resize', () => {
    setupCanvasSize();
    if (state.isFitMode && state.imgObj) {
      fitToWindow();
    }
    drawImage();
  });
  console.log('imsViewer 前端初始化完成');
}

// ===== Canvas 尺寸适配（支持高 DPI） =====
let dpr = window.devicePixelRatio || 1;

function setupCanvasSize() {
  const rect = $('#center-panel').getBoundingClientRect();
  const cw = rect.width, ch = rect.height;
  dom.canvas.width = cw * dpr;
  dom.canvas.height = ch * dpr;
  dom.canvas.style.width = cw + 'px';
  dom.canvas.style.height = ch + 'px';
}

// ===== 格式映射 =====
const FORMAT_MIME = ['image/jpeg', 'image/png', 'image/webp', 'image/avif'];
const FORMAT_LABEL = ['JPEG', 'PNG', 'WebP', 'AVIF'];

function formatMime(fmt) { return FORMAT_MIME[fmt] || 'image/jpeg'; }
function formatLabel(fmt) { return FORMAT_LABEL[fmt] || '未知'; }

// ===== base64 转 Uint8Array =====
function base64ToBytes(b64) {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

// ===== 图片显示 =====
function loadCurrentImage() {
  if (!state.subImages.length || state.currentSubIdx >= state.subImages.length) return;
  const sub = state.subImages[state.currentSubIdx];
  console.log('加载图片:', sub.filename, 'index:', state.currentSubIdx);

  const mime = formatMime(sub.format);
  const rawBytes = base64ToBytes(sub.bytes_base64);
  const blob = new Blob([rawBytes], { type: mime });
  const url = URL.createObjectURL(blob);

  if (state.imgObj && state.imgObj._url) URL.revokeObjectURL(state.imgObj._url);

  const img = new Image();
  img._url = url;
  img.onload = () => {
    state.imgObj = img;
    fitToWindow();
    drawImage();
    updateCurrentImageInfo(sub);
    updateStatusBar();
  };
  img.onerror = (err) => {
    console.error('图片加载失败:', err);
  };
  img.src = url;
}

function cssWidth() { return dom.canvas.width / dpr; }
function cssHeight() { return dom.canvas.height / dpr; }

function fitToWindow() {
  if (!state.imgObj) return;
  const cw = cssWidth();
  const ch = cssHeight();
  const sx = cw / state.imgObj.width;
  const sy = ch / state.imgObj.height;
  state.scale = Math.min(sx, sy);
  state.offsetX = (cw - state.imgObj.width * state.scale) / 2;
  state.offsetY = (ch - state.imgObj.height * state.scale) / 2;
  state.isFitMode = true;
  updateZoomLabel();
}

function centerOriginal() {
  if (!state.imgObj) return;
  state.scale = 1.0 / dpr;  // 1 image pixel = 1 physical screen pixel
  state.offsetX = (cssWidth() - state.imgObj.width * state.scale) / 2;
  state.offsetY = (cssHeight() - state.imgObj.height * state.scale) / 2;
  state.isFitMode = false;
  updateZoomLabel();
}

function drawImage() {
  if (!state.imgObj) return;
  const ctx = dom.ctx;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, dom.canvas.width, dom.canvas.height);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.translate(state.offsetX, state.offsetY);
  ctx.scale(state.scale, state.scale);
  ctx.drawImage(state.imgObj, 0, 0);
}

function zoomAt(factor, cx, cy) {
  const oldScale = state.scale;
  state.scale = Math.max(0.05 / dpr, Math.min(20 / dpr, state.scale * factor));
  state.offsetX = cx - (cx - state.offsetX) * (state.scale / oldScale);
  state.offsetY = cy - (cy - state.offsetY) * (state.scale / oldScale);
  state.isFitMode = false;
  updateZoomLabel();
  drawImage();
}

// 显示物理缩放比例（相对于屏幕像素）
function updateZoomLabel() {
  dom.statusZoom.textContent = Math.round(state.scale * dpr * 100) + '%';
}

// ===== 缩略图 =====
function buildThumbnails() {
  dom.thumbnailList.innerHTML = '';
  const isIms = state.imsInfo !== null;

  if (isIms) {
    dom.thumbnailHeader.textContent = `子图列表 (${state.subImages.length})`;
    state.subImages.forEach((sub, i) => {
      dom.thumbnailList.appendChild(createThumbItem(sub, i));
    });
  } else {
    dom.thumbnailHeader.textContent = '单张图片';
    if (state.subImages.length > 0) {
      dom.thumbnailList.appendChild(createThumbItem(state.subImages[0], 0));
    }
  }
  highlightThumbnail(state.currentSubIdx);
}

function createThumbItem(sub, index) {
  const mime = formatMime(sub.format);
  const rawBytes = base64ToBytes(sub.bytes_base64);
  const blob = new Blob([rawBytes], { type: mime });
  const url = URL.createObjectURL(blob);

  const div = document.createElement('div');
  div.className = 'thumb-item';
  div.dataset.index = index;
  div.innerHTML = `
    <img src="${url}" alt="${sub.filename}" />
    <span class="thumb-label">${sub.filename}</span>
    ${sub.isCover ? '<span class="cover-badge">⭐</span>' : ''}
  `;
  div.addEventListener('click', () => selectSubImage(index));
  return div;
}

function highlightThumbnail(index) {
  $$('.thumb-item').forEach((el, i) => {
    el.classList.toggle('active', i === index);
  });
  const active = dom.thumbnailList.querySelector('.thumb-item.active');
  if (active) active.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

function selectSubImage(index) {
  if (index < 0 || index >= state.subImages.length) return;
  state.currentSubIdx = index;
  loadCurrentImage();
  highlightThumbnail(index);
  updateCurrentImageInfo(state.subImages[index]);
}

// ===== 文件导航 =====
async function openFile(filepath) {
  console.log('openFile 被调用, path:', filepath);
  try {
    const payload = await invoke('open_file', { path: filepath });
    console.log('open_file 返回 payload:', payload);

    state.folderList = payload.folder_list;
    state.currentFileIdx = payload.file_index;
    state.imsInfo = payload.ims_info;
    state.subImages = payload.sub_images;
    state.currentSubIdx = 0;

    buildThumbnails();
    loadCurrentImage();
    updateFileInfo(payload);
    updateFolderBar();
    updateStatusBar();

    const fname = state.folderList[state.currentFileIdx]?.filename ?? filepath;
    document.title = `imsViewer - ${fname}`;
    dom.dropHint.classList.remove('visible');
  } catch (err) {
    console.error('打开文件失败:', err);
    alert('打开文件失败: ' + err);
  }
}

async function navigateFile(delta) {
  if (state.folderList.length === 0) return;
  let newIdx = state.currentFileIdx + delta;
  if (newIdx < 0) newIdx = state.folderList.length - 1;
  if (newIdx >= state.folderList.length) newIdx = 0;
  await openFile(state.folderList[newIdx].filepath);
}

async function navigateSub(delta) {
  if (!state.imsInfo || state.subImages.length === 0) return;
  let newIdx = state.currentSubIdx + delta;
  if (newIdx < 0) newIdx = state.subImages.length - 1;
  if (newIdx >= state.subImages.length) newIdx = 0;
  selectSubImage(newIdx);
}

// ===== 文件夹导航条 =====
function setupFolderBar() {
  $('#btn-prev-file').addEventListener('click', () => navigateFile(-1));
  $('#btn-next-file').addEventListener('click', () => navigateFile(1));
}

function updateFolderBar() {
  if (state.folderList.length === 0) {
    dom.folderPosition.textContent = '--';
    return;
  }
  dom.folderPosition.textContent = `${state.currentFileIdx + 1} / ${state.folderList.length}`;
}

// ===== 信息面板 =====
function updateFileInfo(payload) {
  dom.infoFileName.textContent = payload.filename ?? '--';
  dom.infoFileType.textContent = payload.is_ims ? 'IMS 容器' : payload.file_type ?? '--';
  dom.infoFileSize.textContent = formatSize(payload.file_size ?? 0);

  const isIms = payload.ims_info !== null;
  toggleImsRows(isIms);

  if (isIms) {
    const info = payload.ims_info;
    dom.infoImsVersion.textContent = 'v' + info.version;
    dom.infoImageCount.textContent = info.image_count;
    dom.infoCoverFile.textContent = info.images.find(i => i.is_cover)?.filename ?? '--';
  }
}

function updateCurrentImageInfo(sub) {
  if (!sub) {
    dom.infoImageName.textContent = '--';
    dom.infoImageFormat.textContent = '--';
    dom.infoImageResolution.textContent = '--';
    dom.infoImageSize.textContent = '--';
    dom.infoImageOffset.textContent = '--';
    return;
  }
  dom.infoImageName.textContent = sub.filename ?? '--';
  dom.infoImageFormat.textContent = formatLabel(sub.format);
  dom.infoImageSize.textContent = formatSize(sub.data_length ?? 0);

  if (state.imgObj && state.currentSubIdx === (sub.index ?? 0)) {
    dom.infoImageResolution.textContent = `${state.imgObj.width} × ${state.imgObj.height}`;
  } else {
    dom.infoImageResolution.textContent = '--';
  }
  dom.infoImageOffset.textContent = sub.data_offset !== undefined ? String(sub.data_offset) : '--';
  const hash = sub.hash || '--';
  dom.infoImageHash.textContent = hash.length > 20 ? hash.substring(0, 20) + '...' : hash;
  dom.infoImageHash.title = hash; // 悬浮看完整 hash
}

function toggleImsRows(visible) {
  const rows = ['#row-ims-version', '#row-image-count', '#row-cover-file', '#row-image-offset', '#row-image-hash'];
  rows.forEach(sel => {
    const el = $(sel);
    if (el) el.style.display = visible ? '' : 'none';
  });
}

function updateStatusBar() {
  if (state.folderList.length === 0) {
    dom.statusFilePos.textContent = '未打开文件';
    dom.statusSubPos.textContent = '--';
    return;
  }
  const fname = state.folderList[state.currentFileIdx]?.filename ?? '--';
  dom.statusFilePos.textContent = `文件 ${state.currentFileIdx + 1}/${state.folderList.length}: ${fname}`;

  if (state.imsInfo) {
    dom.statusSubPos.textContent = `子图 ${state.currentSubIdx + 1}/${state.subImages.length}`;
  } else {
    dom.statusSubPos.textContent = '单张';
  }
}

// ===== 键盘事件 =====
function setupKeyboard() {
  document.addEventListener('keydown', (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;

    switch (e.key) {
      case 'ArrowLeft':  e.preventDefault(); navigateFile(-1); break;
      case 'ArrowRight': e.preventDefault(); navigateFile(1);  break;
      case 'ArrowUp':    e.preventDefault(); navigateSub(-1);  break;
      case 'ArrowDown':  e.preventDefault(); navigateSub(1);   break;
      case 'Home':       e.preventDefault(); navigateFile(-state.currentFileIdx); break;
      case 'End':        e.preventDefault(); navigateFile(state.folderList.length - 1 - state.currentFileIdx); break;
      case '0':          e.preventDefault(); fitToWindow(); drawImage(); break;
      case '1':          e.preventDefault(); centerOriginal(); drawImage(); break;
      case '+':
      case '=':          e.preventDefault(); zoomAt(1.25, dom.canvas.width / 2, dom.canvas.height / 2); break;
      case '-':          e.preventDefault(); zoomAt(0.8, dom.canvas.width / 2, dom.canvas.height / 2); break;
      case 'o':
        if (e.ctrlKey) {
          e.preventDefault();
          pickAndOpenFile();
        }
        break;
      case 'e':
        if (e.ctrlKey && e.shiftKey) {
          e.preventDefault();
          exportAll();
        } else if (e.ctrlKey) {
          e.preventDefault();
          exportCurrent();
        }
        break;
    }
  });
}

// ===== 鼠标事件 =====
function setupMouse() {
  let isPanning = false;
  let panStartX = 0, panStartY = 0;
  let panOrigOffsetX = 0, panOrigOffsetY = 0;

  dom.canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    if (e.ctrlKey || e.metaKey) {
      // Ctrl+滚轮：以鼠标位置为中心缩放
      const rect = dom.canvas.getBoundingClientRect();
      const factor = e.deltaY < 0 ? 1.15 : 0.87;
      zoomAt(factor, e.clientX - rect.left, e.clientY - rect.top);
    } else if (e.shiftKey) {
      // Shift+滚轮：切换子图
      navigateSub(e.deltaY > 0 ? 1 : -1);
    } else {
      // 直接滚轮：切换文件
      navigateFile(e.deltaY > 0 ? 1 : -1);
    }
  });

  dom.canvas.addEventListener('mousedown', (e) => {
    if (e.button === 0) {
      isPanning = true;
      panStartX = e.clientX;
      panStartY = e.clientY;
      panOrigOffsetX = state.offsetX;
      panOrigOffsetY = state.offsetY;
    }
  });

  window.addEventListener('mousemove', (e) => {
    if (!isPanning) return;
    state.offsetX = panOrigOffsetX + (e.clientX - panStartX);
    state.offsetY = panOrigOffsetY + (e.clientY - panStartY);
    state.isFitMode = false;
    drawImage();
  });

  window.addEventListener('mouseup', () => { isPanning = false; });

  dom.canvas.addEventListener('dblclick', () => {
    if (state.scale < 0.99) {
      fitToWindow(); // 内部设置 isFitMode = true
    } else {
      centerOriginal();
    }
    drawImage();
    updateZoomLabel();
  });
}

// ===== Tauri 原生拖放（代替 HTML5 DataTransfer） =====
async function setupNativeDrop() {
  try {
    const appWindow = getCurrentWindow();
    console.log('注册 Tauri 原生拖放监听...');

    await appWindow.onDragDropEvent((event) => {
      console.log('拖放事件:', event.payload.type);
      const { type, paths } = event.payload;

      if (type === 'over') {
        dom.dropHint.classList.add('visible');
      } else if (type === 'leave') {
        dom.dropHint.classList.remove('visible');
      } else if (type === 'drop') {
        dom.dropHint.classList.remove('visible');
        if (paths && paths.length > 0) {
          const filepath = paths[0];
          console.log('拖放文件:', filepath);
          openFile(filepath);
        }
      }
    });
    console.log('拖放监听注册成功');
  } catch (err) {
    console.error('拖放监听注册失败:', err);
    // 回退到 HTML5 拖放
    setupHtml5Drop();
  }
}

// HTML5 拖放回退方案
function setupHtml5Drop() {
  console.log('使用 HTML5 拖放回退方案');
  const panel = $('#center-panel');
  panel.addEventListener('dragover', (e) => {
    e.preventDefault();
    e.stopPropagation();
    dom.dropHint.classList.add('visible');
  });
  panel.addEventListener('dragleave', () => {
    dom.dropHint.classList.remove('visible');
  });
  panel.addEventListener('drop', async (e) => {
    e.preventDefault();
    e.stopPropagation();
    dom.dropHint.classList.remove('visible');

    // 尝试从 dataTransfer 获取文件路径
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      const path = files[0].path || files[0].name;
      console.log('HTML5 拖放文件:', path);
      if (path) await openFile(path);
    }
  });
}

// ===== 菜单栏 =====
function setupMenuActions() {
  $$('.menu-action').forEach(el => {
    el.addEventListener('click', async () => {
      const action = el.dataset.action;
      console.log('菜单操作:', action);
      switch (action) {
        case 'open':
          await pickAndOpenFile();
          break;
        case 'export-current':
          exportCurrent();
          break;
        case 'export-all':
          exportAll();
          break;
        case 'fit':
          fitToWindow();
          drawImage();
          break;
        case 'original':
          centerOriginal();
          drawImage();
          break;
        case 'zoom-in':
          zoomAt(1.25, dom.canvas.width / 2, dom.canvas.height / 2);
          break;
        case 'zoom-out':
          zoomAt(0.8, dom.canvas.width / 2, dom.canvas.height / 2);
          break;
        case 'batch-convert':
          openBatchModal();
          break;
        case 'batch-upgrade':
          openUpgradeModal();
          break;
        case 'register-assoc':
          registerAssociations();
          break;
        case 'help-shortcuts':
          alert(
            '═══ 键盘快捷键 ═══\n\n' +
            '浏览导航:\n' +
            '  ← →  切换上一个/下一个文件\n' +
            '  ↑ ↓  切换上一个/下一个子图 (IMS)\n' +
            '  Home  跳到第一个文件\n' +
            '  End   跳到最后一个文件\n\n' +
            '滚轮:\n' +
            '  滚轮      切换文件\n' +
            '  Shift+滚轮 切换子图\n' +
            '  Ctrl+滚轮  缩放图片\n\n' +
            '缩放:\n' +
            '  + / -  放大 / 缩小\n' +
            '  0      适应窗口\n' +
            '  1      原始大小 (居中)\n' +
            '  双击   适应窗口 ↔ 原始大小\n\n' +
            '操作:\n' +
            '  Ctrl+O       打开文件\n' +
            '  Ctrl+E       导出当前图片\n' +
            '  Ctrl+Shift+E 导出全部子图\n' +
            '  拖放图片      打开文件'
          );
          break;
        case 'help-usage':
          alert(
            '═══ 使用说明 ═══\n\n' +
            '打开图片:\n' +
            '  Ctrl+O 选择文件，或直接拖放图片到窗口\n\n' +
            'IMS 格式:\n' +
            '  文件名含 .ims (如 texture.ims.jpg)\n' +
            '  封面在文件头，子图追加在后\n' +
            '  ↑ ↓ 键切换子图，左侧显示缩略图\n\n' +
            '批量转换:\n' +
            '  工具 → 批量转换\n' +
            '  添加文件夹 → 设置深度 → 开始\n' +
            '  命名规范: 封面 name.jpg, 子图 name_00001_.jpg\n\n' +
            '支持格式:\n' +
            '  JPEG, PNG, WebP, AVIF\n' +
            '  (WebP/AVIF 打包时自动转 PNG)'
          );
          break;
        case 'about':
          alert('imsViewer v1.0\nIMS 图片浏览器\n基于 Tauri v2 + Rust + Canvas');
          break;
      }
    });
  });
}

async function pickAndOpenFile() {
  try {
    console.log('调用 pick_file...');
    const path = await invoke('pick_file');
    console.log('pick_file 返回:', path);
    if (path) await openFile(path);
  } catch (e) {
    console.error('pick_file 失败:', e);
  }
}

// ===== 导出 =====
async function exportCurrent() {
  if (state.folderList.length === 0) return;
  const entry = state.folderList[state.currentFileIdx];
  try {
    const outPath = await invoke('extract_image', {
      path: entry.filepath,
      index: state.currentSubIdx,
    });
    alert('已导出: ' + outPath);
  } catch (err) {
    alert('导出失败: ' + err);
  }
}

async function exportAll() {
  if (state.folderList.length === 0 || !state.imsInfo) return;
  const entry = state.folderList[state.currentFileIdx];
  try {
    const paths = await invoke('extract_all', { path: entry.filepath });
    alert(`已导出 ${paths.length} 张图片`);
  } catch (err) {
    alert('导出失败: ' + err);
  }
}

// ===== 工具函数 =====
function formatSize(bytes) {
  if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + ' MB';
  if (bytes >= 1024) return (bytes / 1024).toFixed(1) + ' KB';
  return bytes + ' bytes';
}

// ===== 批量转换模态框 =====
let batchUnlisten = null;

function openBatchModal() {
  const modal = $('#batch-modal');
  modal.style.display = 'flex';
  $('#folder-list').innerHTML = '';
  addFolderRow();
  $('#progress-fill').style.width = '0%';
  $('#progress-text').textContent = '准备就绪';
  $('#btn-start-convert').disabled = false;
  setupBatchEvents();
}

function closeBatchModal() {
  $('#batch-modal').style.display = 'none';
  if (batchUnlisten) {
    batchUnlisten();
    batchUnlisten = null;
  }
}

function addFolderRow(path = '', depth = 0) {
  const row = document.createElement('div');
  row.className = 'folder-row';
  row.innerHTML = `
    <input class="path-input" type="text" placeholder="文件夹路径..." value="${escapeHtml(path)}" />
    <button class="btn-browse">浏览</button>
    <label title="0=无限深度">深度 <input class="depth-input" type="number" min="0" max="100" value="${depth}" /></label>
    <button class="btn-remove" title="移除">×</button>
  `;
  row.querySelector('.btn-browse').addEventListener('click', async () => {
    try {
      const dir = await invoke('pick_folder');
      if (dir) row.querySelector('.path-input').value = dir;
    } catch (e) { console.error('选择文件夹失败:', e); }
  });
  row.querySelector('.btn-remove').addEventListener('click', () => {
    row.remove();
    if ($('#folder-list').children.length === 0) addFolderRow();
  });
  $('#folder-list').appendChild(row);
}

function collectBatchConfig() {
  const folders = [];
  $$('#folder-list .folder-row').forEach(row => {
    const path = row.querySelector('.path-input').value.trim();
    const depth = parseInt(row.querySelector('.depth-input').value) || 0;
    if (path) folders.push({ path, depth: Math.min(100, Math.max(0, depth)) });
  });
  const outputDir = $('#output-dir-input').value.trim();
  return {
    folders,
    delete_source: $('#chk-delete-source').checked,
    multi_threaded: $('#chk-multi-thread').checked,
    output_dir: outputDir || null,
    preserve_structure: $('#chk-preserve-structure').checked,
  };
}

async function startBatchConvert() {
  const config = collectBatchConfig();
  if (config.folders.length === 0) {
    alert('请至少添加一个文件夹');
    return;
  }

  const btn = $('#btn-start-convert');
  btn.disabled = true;
  btn.textContent = '转换中...';
  $('#progress-text').textContent = '正在扫描文件...';

  try {
    // 注册进度监听
    if (batchUnlisten) batchUnlisten();
    batchUnlisten = await listen('batch-progress', (event) => {
      const p = event.payload;
      const pct = p.total > 0 ? Math.round((p.current / p.total) * 100) : 0;
      $('#progress-fill').style.width = pct + '%';
      $('#progress-text').textContent = `[${p.current}/${p.total}] ${p.message}`;

      if (p.status === 'completed' || p.status === 'error') {
        btn.disabled = false;
        btn.textContent = '开始转换';
        if (p.status === 'completed') {
          $('#progress-text').textContent = `✅ ${p.message}`;
        } else {
          $('#progress-text').textContent = `⚠️ ${p.message}`;
        }
      }
    });

    await invoke('start_batch_convert', { config });
  } catch (err) {
    alert('启动转换失败: ' + err);
    btn.disabled = false;
    btn.textContent = '开始转换';
  }
}

function setupBatchEvents() {
  $('#btn-add-folder').onclick = () => addFolderRow();
  $('#btn-modal-close').onclick = () => closeBatchModal();
  $('#btn-cancel-convert').onclick = () => closeBatchModal();
  $('#btn-start-convert').onclick = () => startBatchConvert();
  $('#btn-browse-output').onclick = async () => {
    try {
      const dir = await invoke('pick_folder');
      if (dir) $('#output-dir-input').value = dir;
    } catch (e) { console.error('选择文件夹失败:', e); }
  };
  // 默认添加一行空文件夹
  if ($('#folder-list').children.length === 0) addFolderRow();
}

function escapeHtml(str) {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

// ===== 批量升级模态框 =====
let upgradeUnlisten = null;

function openUpgradeModal() {
  const modal = $('#upgrade-modal');
  modal.style.display = 'flex';
  $('#upgrade-progress-fill').style.width = '0%';
  $('#upgrade-progress-text').textContent = '准备就绪';
  $('#btn-start-upgrade').disabled = false;

  $('#btn-browse-upgrade').onclick = async () => {
    try {
      const dir = await invoke('pick_folder');
      if (dir) $('#upgrade-dir-input').value = dir;
    } catch (e) { console.error(e); }
  };
  $('#btn-upgrade-close').onclick = () => closeUpgradeModal();
  $('#btn-cancel-upgrade').onclick = () => closeUpgradeModal();
  $('#btn-start-upgrade').onclick = () => startBatchUpgrade();
}

function closeUpgradeModal() {
  $('#upgrade-modal').style.display = 'none';
  if (upgradeUnlisten) { upgradeUnlisten(); upgradeUnlisten = null; }
}

async function startBatchUpgrade() {
  const path = $('#upgrade-dir-input').value.trim();
  if (!path) { alert('请选择目录'); return; }
  const depth = parseInt($('#upgrade-depth-input').value) || 0;
  const multi = $('#chk-upgrade-multi').checked;

  const btn = $('#btn-start-upgrade');
  btn.disabled = true;
  btn.textContent = '升级中...';
  $('#upgrade-progress-text').textContent = '正在扫描 IMS 文件...';

  try {
    if (upgradeUnlisten) upgradeUnlisten();
    upgradeUnlisten = await listen('batch-progress', (event) => {
      const p = event.payload;
      const pct = p.total > 0 ? Math.round((p.current / p.total) * 100) : 0;
      $('#upgrade-progress-fill').style.width = pct + '%';
      $('#upgrade-progress-text').textContent = `[${p.current}/${p.total}] ${p.message}`;

      if (p.status === 'completed' || p.status === 'error') {
        btn.disabled = false;
        btn.textContent = '开始升级';
        $('#upgrade-progress-text').textContent = (p.status === 'completed' ? '✅ ' : '⚠️ ') + p.message;
      }
    });

    await invoke('start_batch_upgrade', { path, depth, multiThreaded: multi });
  } catch (err) {
    alert('升级失败: ' + err);
    btn.disabled = false;
    btn.textContent = '开始升级';
  }
}

async function registerAssociations() {
  try {
    const msg = await invoke('register_file_associations');
    alert('✅ ' + msg + '\n\n支持的格式: JPG, JPEG, PNG, WebP, AVIF, IMS.JPG, IMS.PNG\n\n右键图片文件 → 打开方式 → imsViewer\n或使用安装包版本自动设置默认关联。');
  } catch (err) {
    alert('注册失败: ' + err + '\n\n请尝试以管理员身份运行 imsViewer 后重试。');
  }
}

// ===== 启动 =====
console.log('Tauri API:', typeof invoke);
init();

// 处理右键"使用 imsViewer 打开"传入的文件路径
(async () => {
  try {
    const path = await invoke('get_launch_file');
    if (path) {
      console.log('启动文件:', path);
      await openFile(path);
    }
  } catch (e) {
    console.error('获取启动文件失败:', e);
  }
})();
