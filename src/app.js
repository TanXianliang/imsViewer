// ============================================================
// imsViewer — 前端主逻辑
// Tauri v2 + Canvas + 三栏布局
// ============================================================

import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { setupDedupTool } from './dedup.js';

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
  preloadCancel: null,      // 取消后台加载标志
  editMode: false,          // 编辑模式
  dirty: false,             // 是否有未保存的修改
  selectedIndices: new Set(), // 编辑模式下选中的子图索引
  dragState: null,           // { fromIndex, el } 拖拽排序状态
  undoStack: [],             // EditCommand[] 撤销栈
  redoStack: [],             // EditCommand[] 重做栈
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
  titleFilename: $('#title-filename'),
};

// ===== 初始化 =====
function init() {
  dom.ctx = dom.canvas.getContext('2d');
  setupCanvasSize();
  setupKeyboard();
  setupMouse();
  setupNativeDrop();
  setupMenuActions();
  setupContextMenu();
  setupThumbDrag();
  setupFolderBar();
  setupWindowControls();
  setupDedupTool({ openFile });

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

const MAX_UNDO_DEPTH = 30;

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
async function loadCurrentImage() {
  if (!state.subImages.length || state.currentSubIdx >= state.subImages.length) return;
  let sub = state.subImages[state.currentSubIdx];
  console.log('加载图片:', sub.filename, 'index:', state.currentSubIdx);

  // 延迟加载：如果当前子图没有数据，从后端获取
  if (!sub.bytes_base64 && state.imsInfo) {
    try {
      sub = await invoke('get_sub_image', {
        path: state.folderList[state.currentFileIdx].filepath,
        index: state.currentSubIdx,
      });
      state.subImages[state.currentSubIdx] = sub;
      updateThumbnailImage(state.currentSubIdx, sub);
    } catch (err) {
      console.error('加载子图失败:', err);
      markThumbnailLoadFailed(state.currentSubIdx);
      return;
    }
  }

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

  if (isIms || state.editMode) {
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
  const div = document.createElement('div');
  div.className = 'thumb-item';
  div.dataset.index = index;

  if (sub.bytes_base64) {
    const mime = formatMime(sub.format);
    const rawBytes = base64ToBytes(sub.bytes_base64);
    const blob = new Blob([rawBytes], { type: mime });
    const url = URL.createObjectURL(blob);
    div.innerHTML = `
      <img src="${url}" alt="${sub.filename}" />
      <span class="thumb-label">${sub.filename}</span>
      ${sub.isCover ? '<span class="cover-badge">⭐</span>' : ''}
    `;
  } else {
    div.innerHTML = `
      <div class="thumb-loading"></div>
      <span class="thumb-label">${sub.filename}</span>
      ${sub.isCover ? '<span class="cover-badge">⭐</span>' : ''}
    `;
  }

  // 点击：浏览模式切换子图（编辑模式的点击在 mousedown/move/up 中处理）
  div.addEventListener('click', (e) => {
    if (!state.editMode) selectSubImage(index);
  });

  // 右键菜单（仅编辑模式）
  div.addEventListener('contextmenu', (e) => {
    if (!state.editMode) return;
    e.preventDefault();
    if (!state.selectedIndices.has(index)) {
      state.selectedIndices.clear();
      state.selectedIndices.add(index);
      updateSelectionHighlight();
    }
    showContextMenu(e.clientX, e.clientY, index);
  });

  return div;
}

function updateThumbnailImage(index, sub) {
  const thumbEl = dom.thumbnailList.querySelector(`.thumb-item[data-index="${index}"]`);
  const placeholder = thumbEl?.querySelector('.thumb-loading, .thumb-error');
  if (!placeholder || !sub.bytes_base64) return;

  const mime = formatMime(sub.format);
  const rawBytes = base64ToBytes(sub.bytes_base64);
  const url = URL.createObjectURL(new Blob([rawBytes], { type: mime }));
  const img = document.createElement('img');
  img.src = url;
  img.alt = sub.filename;
  img.onload = () => URL.revokeObjectURL(url);
  img.onerror = () => {
    URL.revokeObjectURL(url);
    const failed = document.createElement('div');
    failed.className = 'thumb-error';
    failed.title = '缩略图加载失败，点击重试';
    failed.textContent = '!';
    img.replaceWith(failed);
  };
  placeholder.replaceWith(img);
}

function markThumbnailLoadFailed(index) {
  const thumbEl = dom.thumbnailList.querySelector(`.thumb-item[data-index="${index}"]`);
  const placeholder = thumbEl?.querySelector('.thumb-loading');
  if (!placeholder) return;

  const failed = document.createElement('div');
  failed.className = 'thumb-error';
  failed.title = '缩略图加载失败，点击重试';
  failed.textContent = '!';
  placeholder.replaceWith(failed);
}

// ===== 缩略图拖拽 + 编辑模式点击（绑定在 thumbnail-list 上委托） =====
function setupThumbDrag() {
  const list = $('#thumbnail-list');
  let dragIdx = -1, startX = 0, startY = 0, moved = false, clone = null;

  list.addEventListener('mousedown', (e) => {
    if (!state.editMode || e.button !== 0) return;
    const thumb = e.target.closest('.thumb-item');
    if (!thumb) return;
    dragIdx = parseInt(thumb.dataset.index);
    if (isNaN(dragIdx)) { dragIdx = -1; return; }
    startX = e.clientX;
    startY = e.clientY;
    moved = false;
    e.preventDefault();
  });

  let insertIdx = -1;

  window.addEventListener('mousemove', (e) => {
    if (dragIdx < 0) return;
    if (!moved && Math.abs(e.clientX - startX) < 5 && Math.abs(e.clientY - startY) < 5) return;
    if (!moved) {
      moved = true;
      const el = list.querySelector(`.thumb-item[data-index="${dragIdx}"]`);
      if (!el) { dragIdx = -1; return; }
      el.classList.add('dragging');
      clone = el.cloneNode(true);
      clone.style.cssText = `position:fixed;z-index:9999;pointer-events:none;opacity:0.85;transform:rotate(3deg) scale(1.05);width:${el.offsetWidth}px;`;
      document.body.appendChild(clone);
    }
    clone.style.left = (e.clientX - clone.offsetWidth / 2) + 'px';
    clone.style.top = (e.clientY - clone.offsetHeight / 2) + 'px';
    $$('.thumb-item').forEach(el => { el.style.borderTop = ''; el.style.borderBottom = ''; });
    insertIdx = -1;
    if (clone) clone.style.display = 'none';
    const t = document.elementFromPoint(e.clientX, e.clientY)?.closest('.thumb-item');
    if (clone) clone.style.display = '';
    if (t) {
      const ti = parseInt(t.dataset.index);
      if (!isNaN(ti) && ti !== dragIdx && ti !== 0) {
        const rect = t.getBoundingClientRect();
        if (e.clientY < rect.top + rect.height / 2) {
          t.style.borderTop = '3px solid #2ecc71';
          insertIdx = ti;
        } else {
          t.style.borderBottom = '3px solid #2ecc71';
          insertIdx = ti + 1;
        }
      }
    }
    if (insertIdx < 0 && list.children.length > 0) {
      const last = list.lastElementChild;
      if (last) {
        const lr = last.getBoundingClientRect();
        if (e.clientY > lr.bottom - 5 && e.clientX > lr.left && e.clientX < lr.right) {
          last.style.borderBottom = '3px solid #2ecc71';
          insertIdx = state.subImages.length;
        }
      }
    }
  });

  // 右键取消拖拽
  window.addEventListener('mousedown', (e) => {
    if (e.button === 2 && dragIdx >= 0) { cleanupDrag(); }
  });
  document.addEventListener('contextmenu', (e) => {
    if (dragIdx >= 0 || moved) { e.preventDefault(); e.stopPropagation(); }
  }, true);

  function cleanupDrag() {
    $$('.thumb-item').forEach(el => { el.classList.remove('dragging'); el.style.borderTop = ''; el.style.borderBottom = ''; });
    if (clone) { clone.remove(); clone = null; }
    dragIdx = -1;
    moved = false;
    insertIdx = -1;
  }

  document.addEventListener('mouseleave', cleanupDrag);
  window.addEventListener('blur', cleanupDrag);

  window.addEventListener('mouseup', (e) => {
    if (dragIdx < 0) return;
    if (moved) {
      // 封面不可拖拽移动
      if (insertIdx >= 1 && dragIdx !== 0) {
        const toIndex = dragIdx < insertIdx ? insertIdx - 1 : insertIdx;
        const cmd = new MoveCommand(dragIdx, toIndex);
        cmd.execute(state);
        pushCommand(cmd);
        buildThumbnails();
        updateSelectionHighlight();
      }
    } else {
      // 没有拖拽 = 点击选择
      if (e.ctrlKey || e.metaKey) {
        if (state.selectedIndices.has(dragIdx)) state.selectedIndices.delete(dragIdx);
        else state.selectedIndices.add(dragIdx);
      } else if (e.shiftKey && state.selectedIndices.size > 0) {
        const sel = [...state.selectedIndices];
        const min = Math.min(...sel, dragIdx), max = Math.max(...sel, dragIdx);
        state.selectedIndices.clear();
        for (let i = min; i <= max; i++) state.selectedIndices.add(i);
      } else {
        state.selectedIndices.clear();
        state.selectedIndices.add(dragIdx);
      }
      updateSelectionHighlight();
      selectSubImage(dragIdx);
    }
    cleanupDrag();
  });
}

function handleThumbClick(e, index) {
  if (e.ctrlKey || e.metaKey) {
    // Ctrl+点击：切换选中
    if (state.selectedIndices.has(index)) {
      state.selectedIndices.delete(index);
    } else {
      state.selectedIndices.add(index);
    }
  } else if (e.shiftKey && state.selectedIndices.size > 0) {
    // Shift+点击：范围选中
    const selected = [...state.selectedIndices];
    const min = Math.min(...selected, index);
    const max = Math.max(...selected, index);
    state.selectedIndices.clear();
    for (let i = min; i <= max; i++) state.selectedIndices.add(i);
  } else {
    // 普通点击：单选
    state.selectedIndices.clear();
    state.selectedIndices.add(index);
  }
  updateSelectionHighlight();
  if (state.selectedIndices.size === 1) {
    selectSubImage(index);
  }
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

    // 后台预加载其余子图
    startBackgroundPreload();
    updateFolderBar();
    updateStatusBar();

    const fname = state.folderList[state.currentFileIdx]?.filename ?? filepath;
    document.title = `imsViewer - ${fname}`;
    updateTitleFilename(fname);
    dom.dropHint.classList.remove('visible');

    // 显示编辑菜单
    $('#menu-edit').style.display = '';
    // 文件切换时自动退出编辑模式
    if (state.editMode) await exitEditMode(false);
  } catch (err) {
    console.error('打开文件失败:', err);
    alert('打开文件失败: ' + err);
  }
}

// 关闭当前文件
async function closeCurrentFile() {
  if (state.editMode) await exitEditMode(false);
  state.folderList = [];
  state.currentFileIdx = -1;
  state.imsInfo = null;
  state.subImages = [];
  state.currentSubIdx = 0;
  state.imgObj = null;
  state.scale = 1.0;
  state.offsetX = 0;
  state.offsetY = 0;
  dom.thumbnailList.innerHTML = '';
  dom.thumbnailHeader.textContent = '子图列表';
  dom.folderPosition.textContent = '--';
  dom.statusFilePos.textContent = '未打开文件';
  dom.statusSubPos.textContent = '--';
  dom.titleFilename.textContent = '';
  document.title = 'imsViewer';
  dom.dropHint.classList.add('visible');
  dom.ctx.clearRect(0, 0, dom.canvas.width, dom.canvas.height);
  $('#edit-mode-bar').style.display = 'none';
  updateFileInfo({ filename: '--', file_type: '--', file_size: 0, is_ims: false });
  setStatus('已关闭');
}

async function navigateFile(delta) {
  if (state.editMode || state.folderList.length === 0) return;

  // 先检查目录是否有变更，有则刷新列表
  const activePath = state.folderList[state.currentFileIdx]?.filepath;
  if (activePath) {
    const dir = activePath.replace(/[/\\][^/\\]*$/, '');
    try {
      const freshList = await invoke('refresh_folder', { dir });
      if (freshList && freshList.length > 0) {
        state.folderList = freshList;
        state.currentFileIdx = freshList.findIndex(e => e.filepath === activePath);
        if (state.currentFileIdx < 0) state.currentFileIdx = 0;
        updateFolderBar();
        updateStatusBar();
      }
    } catch (e) {
      console.error('刷新文件夹失败:', e);
    }
  }

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

function setStatus(msg) {
  dom.statusSubPos.textContent = msg;
}

function updateTitleFilename(fname) {
  dom.titleFilename.textContent = fname || '';
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
  document.addEventListener('keydown', async (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;

    switch (e.key) {
      case 'ArrowLeft':  if (!state.editMode) { e.preventDefault(); navigateFile(-1); } break;
      case 'ArrowRight': if (!state.editMode) { e.preventDefault(); navigateFile(1); }  break;
      case 'ArrowUp':    e.preventDefault(); navigateSub(-1);  break;
      case 'ArrowDown':  e.preventDefault(); navigateSub(1);   break;
      case 'Home':       if (!state.editMode) { e.preventDefault(); navigateFile(-state.currentFileIdx); } break;
      case 'End':        if (!state.editMode) { e.preventDefault(); navigateFile(state.folderList.length - 1 - state.currentFileIdx); } break;
      case '0':          e.preventDefault(); fitToWindow(); drawImage(); break;
      case '1':          e.preventDefault(); centerOriginal(); drawImage(); break;
      case '+':
      case '=':          e.preventDefault(); zoomAt(1.25, dom.canvas.width / 2, dom.canvas.height / 2); break;
      case '-':          e.preventDefault(); zoomAt(0.8, dom.canvas.width / 2, dom.canvas.height / 2); break;
      case 'Delete':
        if (state.editMode) { e.preventDefault(); deleteSelectedImages(); }
        break;
      case 'Escape':
        if (state.editMode) { e.preventDefault(); toggleEditMode(); }
        break;
      case 'z':
      case 'Z':
        if (e.ctrlKey && e.shiftKey && state.editMode) {
          e.preventDefault();
          redo();  // Ctrl+Shift+Z = 重做
        } else if (e.ctrlKey && !e.shiftKey && state.editMode) {
          e.preventDefault();
          undo();  // Ctrl+Z = 撤销
        }
        break;
      case 'y':
      case 'Y':
        if (e.ctrlKey && state.editMode) {
          e.preventDefault();
          redo();  // Ctrl+Y = 重做
        }
        break;
      case 's':
      case 'S':
        if (e.ctrlKey && state.editMode) {
          e.preventDefault();
          saveFile();
        }
        break;
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

  dom.canvas.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    isPanning = true;
    panStartX = e.clientX;
    panStartY = e.clientY;
    panOrigOffsetX = state.offsetX;
    panOrigOffsetY = state.offsetY;
  });

  window.addEventListener('mousemove', (e) => {
    if (!isPanning) return;
    state.offsetX = panOrigOffsetX + (e.clientX - panStartX);
    state.offsetY = panOrigOffsetY + (e.clientY - panStartY);
    state.isFitMode = false;
    drawImage();
  });

  window.addEventListener('mouseup', () => { isPanning = false; });

  dom.canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    if (e.ctrlKey || e.metaKey) {
      // Ctrl+滚轮：以鼠标位置为中心缩放
      const rect = dom.canvas.getBoundingClientRect();
      const factor = e.deltaY < 0 ? 1.15 : 0.87;
      zoomAt(factor, e.clientX - rect.left, e.clientY - rect.top);
    } else if (e.shiftKey) {
      // Shift+滚轮：切换文件（编辑模式下锁定）
      if (!state.editMode) navigateFile(e.deltaY > 0 ? 1 : -1);
    } else {
      // 直接滚轮：切换子图
      navigateSub(e.deltaY > 0 ? 1 : -1);
    }
  });

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
          if (state.editMode) {
            // 编辑模式：插入到缩略图列表末尾
            insertImageFiles(paths);
          } else {
            openFile(paths[0]);
          }
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

// ===== 窗口控制（全局引用，供 HTML onclick 调用） =====
async function setupWindowControls() {
  const win = getCurrentWindow();
  const maxBtn = $('#btn-maximize');

  // 更新最大化/还原图标
  const updateMaxIcon = async () => {
    const maximized = await win.isMaximized();
    maxBtn.classList.toggle('restore', maximized);
    maxBtn.title = maximized ? '还原' : '最大化';
  };

  // 按钮点击 + 图标更新
  window.__imsViewer = {
    minimize: () => win.minimize(),
    maximize: async () => {
      await win.toggleMaximize();
      updateMaxIcon();
    },
    close: () => win.close(),
    onTitleBarMouseDown: (e) => {
      // 排除按钮、菜单项、下拉框
      if (e.target.closest('button') || e.target.closest('.menu-item') ||
          e.target.closest('.menu-dropdown') || e.target.closest('.menu-action')) {
        return;
      }
      win.startDragging();
    },
  };

  // 初始状态 + 监听窗口变化
  updateMaxIcon();
  win.onResized(updateMaxIcon);
}

// ===== 菜单栏 =====
function setupMenuActions() {
  $$('.menu-item').forEach(item => {
    item.tabIndex = 0;
    const toggleMenu = () => {
      const shouldOpen = !item.classList.contains('open');
      $$('.menu-item.open').forEach(openItem => openItem.classList.remove('open'));
      if (shouldOpen) item.classList.add('open');
    };
    item.addEventListener('click', event => {
      if (event.target.closest('.menu-action')) return;
      toggleMenu();
    });
    item.addEventListener('keydown', event => {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      event.preventDefault();
      toggleMenu();
    });
  });
  document.addEventListener('click', event => {
    if (event.target.closest('.menu-item')) return;
    $$('.menu-item.open').forEach(item => item.classList.remove('open'));
  });

  $$('.menu-action').forEach(el => {
    el.addEventListener('click', async () => {
      $$('.menu-item.open').forEach(item => item.classList.remove('open'));
      const action = el.dataset.action;
      console.log('菜单操作:', action);
      switch (action) {
        case 'open':
          await pickAndOpenFile();
          break;
        case 'close-file':
          closeCurrentFile();
          break;
        case 'export-current':
          exportCurrent();
          break;
        case 'export-all':
          exportAll();
          break;
        case 'batch-extract':
          openExtractModal();
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
        case 'image-dedup':
          window.dispatchEvent(new CustomEvent('imsviewer:open-dedup'));
          break;
        case 'register-assoc':
          registerAssociations();
          break;
        case 'toggle-edit':
          toggleEditMode();
          break;
        case 'save-ims':
          saveFile();
          break;
        case 'save-as':
          saveFileAs();
          break;
        case 'help-shortcuts':
          alert(
            '═══ 键盘快捷键 ═══\n\n' +
            '浏览导航:\n' +
            '  ← →  切换上一个/下一个文件\n' +
            '  ↑ ↓  切换上一个/下一个子图 (IMS)\n' +
            '  Home  跳到第一个文件\n' +
            '  End   跳到最后一个文件\n\n' +
            '编辑模式:\n' +
            '  Ctrl+S  保存\n' +
            '  Ctrl+Z  撤销\n' +
            '  Ctrl+Y  重做\n' +
            '  Del     删除选中\n' +
            '  Esc     退出编辑\n\n' +
            '滚轮:\n' +
            '  滚轮      切换子图\n' +
            '  Shift+滚轮 切换文件\n' +
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
            '批量解压:\n' +
            '  工具 → 批量解压 IMS\n' +
            '  选择输入目录和输出目录，可保持目录结构\n\n' +
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
    merge_existing_ims: $('#chk-merge-existing-ims').checked,
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

// ===== 批量解压模态框 =====
let extractUnlisten = null;

function openExtractModal() {
  const modal = $('#extract-modal');
  modal.style.display = 'flex';
  $('#extract-progress-fill').style.width = '0%';
  $('#extract-progress-text').textContent = '准备就绪';
  $('#btn-start-extract').disabled = false;
  $('#btn-start-extract').textContent = '开始解压';

  $('#btn-browse-extract').onclick = async () => {
    try {
      const dir = await invoke('pick_folder');
      if (dir) $('#extract-dir-input').value = dir;
    } catch (e) { console.error('选择输入目录失败:', e); }
  };
  $('#btn-browse-extract-output').onclick = async () => {
    try {
      const dir = await invoke('pick_folder');
      if (dir) $('#extract-output-dir-input').value = dir;
    } catch (e) { console.error('选择输出目录失败:', e); }
  };
  $('#btn-extract-close').onclick = () => closeExtractModal();
  $('#btn-cancel-extract').onclick = () => closeExtractModal();
  $('#btn-start-extract').onclick = () => startBatchExtract();
}

function closeExtractModal() {
  $('#extract-modal').style.display = 'none';
  if (extractUnlisten) {
    extractUnlisten();
    extractUnlisten = null;
  }
}

async function startBatchExtract() {
  const inputDir = $('#extract-dir-input').value.trim();
  const outputDir = $('#extract-output-dir-input').value.trim();
  if (!inputDir) { alert('请选择输入目录'); return; }
  if (!outputDir) { alert('请选择输出目录'); return; }

  const depth = parseInt($('#extract-depth-input').value) || 0;
  const config = {
    input_dir: inputDir,
    depth: Math.min(100, Math.max(0, depth)),
    output_dir: outputDir,
    preserve_structure: $('#chk-extract-preserve-structure').checked,
    multi_threaded: $('#chk-extract-multi').checked,
  };

  const btn = $('#btn-start-extract');
  btn.disabled = true;
  btn.textContent = '解压中...';
  $('#extract-progress-text').textContent = '正在扫描 IMS 文件...';

  try {
    if (extractUnlisten) extractUnlisten();
    extractUnlisten = await listen('batch-progress', (event) => {
      const p = event.payload;
      const pct = p.total > 0 ? Math.round((p.current / p.total) * 100) : 0;
      $('#extract-progress-fill').style.width = pct + '%';
      $('#extract-progress-text').textContent = `[${p.current}/${p.total}] ${p.message}`;

      if (p.status === 'completed' || p.status === 'error') {
        btn.disabled = false;
        btn.textContent = '开始解压';
        $('#extract-progress-text').textContent = (p.status === 'completed' ? '✅ ' : '⚠️ ') + p.message;
      }
    });

    await invoke('start_batch_extract', { config });
  } catch (err) {
    alert('启动解压失败: ' + err);
    btn.disabled = false;
    btn.textContent = '开始解压';
  }
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

// ===== Undo/Redo 命令模式 =====
class EditCommand {
  execute(state) {}  // 正向执行/重做
  undo(state) {}     // 撤销
}

// 删除命令：按降序存储索引和对应图片数据
class DeleteCommand extends EditCommand {
  constructor(indices, images) {
    super();
    // indices 已是降序，images 是对应的 SubImagePayload
    this.items = indices.map((idx, i) => ({ index: idx, image: images[i] }));
  }

  execute(state) {
    for (const { index } of this.items) {
      state.subImages.splice(index, 1);
    }
  }

  undo(state) {
    // 升序插回，恢复原始位置
    const ascending = [...this.items].sort((a, b) => a.index - b.index);
    for (const { index, image } of ascending) {
      state.subImages.splice(index, 0, image);
    }
  }
}

// 替换命令
class ReplaceCommand extends EditCommand {
  constructor(index, oldImage, newImage) {
    super();
    this.index = index;
    this.oldImage = oldImage;
    this.newImage = newImage;
  }

  execute(state) {
    state.subImages[this.index] = this.newImage;
  }

  undo(state) {
    state.subImages[this.index] = this.oldImage;
  }
}

// 插入命令
class InsertCommand extends EditCommand {
  constructor(insertAt, images) {
    super();
    this.insertAt = insertAt;
    this.images = images;
  }

  execute(state) {
    state.subImages.splice(this.insertAt, 0, ...this.images);
  }

  undo(state) {
    state.subImages.splice(this.insertAt, this.images.length);
  }
}

// 移动命令（拖拽排序）
class MoveCommand extends EditCommand {
  constructor(fromIndex, toIndex) {
    super();
    this.fromIndex = fromIndex;
    this.toIndex = toIndex;
  }

  execute(state) {
    const [item] = state.subImages.splice(this.fromIndex, 1);
    state.subImages.splice(this.toIndex, 0, item);
  }

  undo(state) {
    const [item] = state.subImages.splice(this.toIndex, 1);
    state.subImages.splice(this.fromIndex, 0, item);
  }
}

// ===== Undo/Redo 栈管理 =====
function pushCommand(cmd) {
  state.redoStack = [];  // 新编辑清空重做栈
  state.undoStack.push(cmd);
  if (state.undoStack.length > MAX_UNDO_DEPTH) {
    state.undoStack.shift();  // 淘汰最旧命令
  }
  state.dirty = state.undoStack.length > 0;
}

function undo() {
  if (!state.editMode || state.undoStack.length === 0) return;
  const cmd = state.undoStack.pop();
  cmd.undo(state);
  state.redoStack.push(cmd);
  state.dirty = state.undoStack.length > 0;
  refreshAfterCommand();
}

function redo() {
  if (!state.editMode || state.redoStack.length === 0) return;
  const cmd = state.redoStack.pop();
  cmd.execute(state);
  state.undoStack.push(cmd);
  state.dirty = state.undoStack.length > 0;
  refreshAfterCommand();
}

function refreshAfterCommand() {
  state.selectedIndices.clear();
  buildThumbnails();
  updateSelectionHighlight();
  if (state.currentSubIdx >= state.subImages.length) {
    state.currentSubIdx = Math.max(0, state.subImages.length - 1);
  }
  loadCurrentImage();
}

// ===== 编辑模式 =====
async function toggleEditMode() {
  if (state.editMode) {
    await exitEditMode(true);
  } else {
    enterEditMode();
  }
}

function enterEditMode() {
  state.editMode = true;
  state.dirty = false;
  state.undoStack = [];
  state.redoStack = [];
  $('#edit-mode-bar').style.display = '';
  const btn = document.querySelector('#menu-edit .menu-action[data-action=\"toggle-edit\"]');
  if (btn) btn.textContent = '退出编辑模式';
  state.currentSubIdx = 0;
  selectSubImage(0);
}

function markDirty() { state.dirty = true; }

function updateSelectionHighlight() {
  $$('.thumb-item').forEach((el, i) => {
    el.classList.toggle('selected', state.selectedIndices.has(i));
  });
}

// 右键菜单
function showContextMenu(x, y, index) {
  const menu = $('#thumb-context-menu');
  menu.style.display = '';
  // 视口边界裁剪
  const mw = menu.offsetWidth || 160;
  const mh = menu.offsetHeight || 120;
  if (x + mw > window.innerWidth) x = window.innerWidth - mw - 4;
  if (y + mh > window.innerHeight) y = window.innerHeight - mh - 4;
  menu.style.left = Math.max(0, x) + 'px';
  menu.style.top = Math.max(0, y) + 'px';

  // 根据是否选中封面调整可用操作
  const hasCover = state.selectedIndices.has(0);
  const delBtn = menu.querySelector('[data-action="delete-selected"]');
  if (delBtn) {
    delBtn.style.display = hasCover && state.selectedIndices.size === 1 ? 'none' : '';
    // 若选中集中含封面，标注封面不会被删
    if (hasCover && state.selectedIndices.size > 1) {
      delBtn.textContent = `删除选中 (${state.selectedIndices.size - 1}张, 封面除外)`;
    } else {
      delBtn.textContent = '删除选中';
    }
  }
}

function setupContextMenu() {
  $$('#thumb-context-menu .context-action').forEach(el => {
    el.addEventListener('click', () => handleContextAction(el.dataset.action));
  });
  document.addEventListener('click', (e) => {
    if (!e.target.closest('#thumb-context-menu')) hideContextMenu();
  });
}

function hideContextMenu() {
  $('#thumb-context-menu').style.display = 'none';
}

// 上下文菜单操作
async function handleContextAction(action) {
  hideContextMenu();
  switch (action) {
    case 'delete-selected':
      deleteSelectedImages();
      break;
    case 'replace-image':
      await replaceSelectedImage();
      break;
    case 'insert-image':
      await insertImages();
      break;
  }
}

// 删除选中子图
function deleteSelectedImages() {
  if (!state.editMode) return;
  const toDelete = [...state.selectedIndices].filter(i => i !== 0); // 封面除外
  if (toDelete.length === 0) return;

  // 从后往前删，避免索引错乱
  toDelete.sort((a, b) => b - a);
  const removedImages = toDelete.map(i => state.subImages[i]);
  const cmd = new DeleteCommand(toDelete, removedImages);
  cmd.execute(state);
  pushCommand(cmd);
  state.selectedIndices.clear();
  buildThumbnails();
  if (state.currentSubIdx >= state.subImages.length) {
    state.currentSubIdx = state.subImages.length - 1;
  }
  loadCurrentImage();
}

// 替换选中图片
async function replaceSelectedImage() {
  if (!state.editMode || state.selectedIndices.size !== 1) return;
  const idx = [...state.selectedIndices][0];
  try {
    const path = await invoke('pick_file');
    if (!path) return;
    const payload = await invoke('open_file', { path });
    if (payload.sub_images.length > 0) {
      const oldSub = state.subImages[idx];  // 捕获旧数据供撤销
      const newSub = payload.sub_images[0];
      if (idx === 0) newSub.isCover = true; // 保持封面标记
      const cmd = new ReplaceCommand(idx, oldSub, newSub);
      cmd.execute(state);
      pushCommand(cmd);
      buildThumbnails();
      updateSelectionHighlight();
      if (idx === state.currentSubIdx) loadCurrentImage();
    }
  } catch (e) {
    console.error('替换失败:', e);
  }
}

// 从文件路径列表批量插入图片
async function insertImageFiles(filepaths) {
  if (!state.editMode) return;
  const insertAt = state.subImages.length;
  const allImages = [];
  for (const fp of filepaths) {
    try {
      const payload = await invoke('open_file', { path: fp });
      if (payload.sub_images.length > 0) {
        for (const sub of payload.sub_images) {
          sub.isCover = false;
          // IMS 子图延迟加载 → 主动加载缺失数据
          if (!sub.bytes_base64 && payload.ims_info) {
            try {
              const loaded = await invoke('get_sub_image', { path: fp, index: sub.index });
              sub.bytes_base64 = loaded.bytes_base64;
              sub.format = loaded.format;
            } catch (e) { console.error('加载子图失败:', sub.filename, e); }
          }
        }
        allImages.push(...payload.sub_images);
      }
    } catch (e) { console.error('插入失败:', fp, e); }
  }
  if (allImages.length > 0) {
    const cmd = new InsertCommand(insertAt, allImages);
    cmd.execute(state);
    pushCommand(cmd);
  }
  buildThumbnails();
  updateSelectionHighlight();
}

// 插入图片（文件对话框）
async function insertImages() {
  if (!state.editMode) return;
  try {
    const path = await invoke('pick_file');
    if (!path) return;
    const payload = await invoke('open_file', { path });
    if (payload.sub_images.length > 0) {
      const insertAt = state.selectedIndices.size === 1
        ? Math.max(1, [...state.selectedIndices][0] + 1) // 封面后，选中项之后
        : state.subImages.length;
      for (const sub of payload.sub_images) {
        sub.isCover = false;
        if (!sub.bytes_base64 && payload.ims_info) {
          try {
            const loaded = await invoke('get_sub_image', { path, index: sub.index });
            sub.bytes_base64 = loaded.bytes_base64;
            sub.format = loaded.format;
          } catch (e) { console.error('加载子图失败:', sub.filename, e); }
        }
      }
      const cmd = new InsertCommand(insertAt, payload.sub_images);
      cmd.execute(state);
      pushCommand(cmd);
      buildThumbnails();
      updateSelectionHighlight();
    }
  } catch (e) {
    console.error('插入失败:', e);
  }
}

// 计算非 IMS 文件保存时的 IMS 输出路径
function computeImsOutputPath(originalPath) {
  const ext = originalPath.split('.').pop().toLowerCase();
  const stem = originalPath.slice(0, originalPath.lastIndexOf('.'));
  // jpg/jpeg → .ims.jpg, png → .ims.png, 其他 → .ims.png
  const outExt = (ext === 'jpg' || ext === 'jpeg') ? 'jpg' : 'png';
  return `${stem}.ims.${outExt}`;
}

// 保存文件（Ctrl+S）
async function saveFile() {
  if (!state.editMode) return;

  const filepath = state.folderList[state.currentFileIdx]?.filepath;
  if (!filepath) return;

  // 非 IMS 文件，无子图 → 不存盘，退出编辑模式
  if (!state.imsInfo && state.subImages.length <= 1) {
    setStatus('无子图片，已退出编辑');
    exitEditMode(false);
    return;
  }

  const savePath = state.imsInfo ? filepath : computeImsOutputPath(filepath);

  try {
    const images = state.subImages.map(sub => ({
      filename: sub.filename,
      format: sub.format,
      bytes_base64: sub.bytes_base64,
      is_cover: sub.is_cover,
    }));

    await invoke('save_ims', { path: savePath, images });

    // 保存成功：清除 dirty 和 undo/redo 栈
    state.dirty = false;
    state.undoStack = [];
    state.redoStack = [];

    // 重新打开文件刷新状态
    if (state.imsInfo) {
      // IMS 覆盖保存：重新打开原路径刷新元数据
      const payload = await invoke('open_file', { path: savePath });
      state.subImages = payload.sub_images;
      state.imsInfo = payload.ims_info;
      buildThumbnails();
      updateSelectionHighlight();
      if (state.currentSubIdx >= state.subImages.length) {
        state.currentSubIdx = Math.max(0, state.subImages.length - 1);
      }
      loadCurrentImage();
      // 重新触发后台预加载，填充非封面子图缩略图
      startBackgroundPreload();
    } else {
      // 非 IMS → IMS：用 openFile 完整刷新（更新文件列表、导航到新文件、退出编辑模式）
      await openFile(savePath);
    }
    setStatus('已保存');
  } catch (e) {
    console.error('保存失败:', e);
    alert('保存失败: ' + e);
  }
}

// 退出确认对话框（保存 / 不保存 / 取消）
function showSaveConfirm() {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'save-confirm-overlay';
    overlay.innerHTML = `
      <div class="save-confirm-box">
        <p>有未保存的修改，是否保存？</p>
        <div class="save-confirm-btns">
          <button class="btn-save">保存</button>
          <button class="btn-discard">不保存</button>
          <button class="btn-cancel">取消</button>
        </div>
      </div>`;
    document.body.appendChild(overlay);

    const cleanup = () => overlay.remove();

    overlay.querySelector('.btn-save').addEventListener('click', () => { cleanup(); resolve('save'); });
    overlay.querySelector('.btn-discard').addEventListener('click', () => { cleanup(); resolve('discard'); });
    overlay.querySelector('.btn-cancel').addEventListener('click', () => { cleanup(); resolve('cancel'); });
    // 点击遮罩背景 = 取消
    overlay.addEventListener('click', (e) => { if (e.target === overlay) { cleanup(); resolve('cancel'); } });
  });
}

// 另存为
async function saveFileAs() {
  if (!state.editMode) return;

  const currentPath = state.folderList[state.currentFileIdx]?.filepath;
  // 默认文件名：IMS 用原名，非 IMS 转 .ims.jpg/.ims.png
  const defaultName = state.imsInfo
    ? (currentPath || 'output').split(/[\\/]/).pop()
    : computeImsOutputPath(currentPath || 'image.jpg').split(/[\\/]/).pop();

  try {
    const savePath = await invoke('pick_save_path', { defaultName });
    if (!savePath) return;

    const images = state.subImages.map(sub => ({
      filename: sub.filename,
      format: sub.format,
      bytes_base64: sub.bytes_base64,
      is_cover: sub.is_cover,
    }));

    await invoke('save_ims', { path: savePath, images });

    state.dirty = false;
    state.undoStack = [];
    state.redoStack = [];
    setStatus('已另存为: ' + savePath.split(/[\\/]/).pop());
  } catch (e) {
    console.error('另存为失败:', e);
    alert('另存为失败: ' + e);
  }
}

let exitingEditMode = false; // 防止退出对话框重复弹出

async function exitEditMode(confirmIfDirty) {
  if (confirmIfDirty && state.dirty) {
    if (exitingEditMode) return; // 已有确认对话框
    exitingEditMode = true;
    const choice = await showSaveConfirm();
    exitingEditMode = false;
    if (choice === 'save') {
      await saveFile();
      // saveFile 对 IMS 保持编辑模式，需额外退出；非 IMS 已通过 openFile 退出
      if (!state.editMode) return;
    } else if (choice === 'cancel') {
      return;
    }
    // 'discard' → 直接退出不保存
  }
  state.editMode = false;
  state.dirty = false;
  state.undoStack = [];
  state.redoStack = [];
  state.selectedIndices.clear();
  $$('.thumb-item').forEach(el => el.classList.remove('selected'));
  $('#edit-mode-bar').style.display = 'none';
  const btn = document.querySelector('#menu-edit .menu-action[data-action=\"toggle-edit\"]');
  if (btn) btn.textContent = '进入编辑模式';
}

// ===== 后台预加载 =====
function startBackgroundPreload() {
  // 取消之前的任务
  state.preloadCancel = true;

  const filepath = state.folderList[state.currentFileIdx]?.filepath;
  if (!filepath || !state.imsInfo) return;

  const cancelToken = {};
  state.preloadCancel = cancelToken;

  const pending = state.subImages
    .map((sub, index) => (sub.bytes_base64 ? -1 : index))
    .filter(index => index >= 0);
  let next = 0;

  const worker = async () => {
    while (state.preloadCancel === cancelToken && next < pending.length) {
      const index = pending[next++];
      if (state.subImages[index].bytes_base64) continue;

      try {
        const loaded = await invoke('get_sub_image', { path: filepath, index });
        if (state.preloadCancel !== cancelToken) return;

        state.subImages[index] = loaded;
        updateThumbnailImage(index, loaded);
      } catch (e) {
        if (state.preloadCancel !== cancelToken) return;
        console.error('预加载子图失败:', index, e);
        markThumbnailLoadFailed(index);
      }
    }
  };

  const workerCount = Math.min(3, pending.length);
  for (let i = 0; i < workerCount; i++) {
    void worker();
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
