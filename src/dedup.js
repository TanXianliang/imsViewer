import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { createDedupBatchController } from './dedup-batch.js';

const PAGE_SIZE = 50;
const THUMBNAIL_CONCURRENCY = 3;

const dedupState = {
  visible: false,
  mode: 'single',
  sourcePath: '',
  galleryDirectories: [],
  activeJobId: null,
  busy: false,
  results: [],
  filteredResults: [],
  page: 0,
  thumbnailGeneration: 0,
  thumbnailQueue: [],
  thumbnailActive: 0,
  modelChecked: false,
};

let openFileCallback = null;
let thumbnailObserver = null;
let batchController = null;

const $ = selector => document.querySelector(selector);

export function setupDedupTool({ openFile }) {
  openFileCallback = openFile;
  initializeBatchController();

  window.addEventListener('imsviewer:open-dedup', showDedupWorkspace);
  $('#dedup-back').addEventListener('click', hideDedupWorkspace);
  $('#dedup-pick-source').addEventListener('click', pickSource);
  $('#dedup-pick-gallery').addEventListener('click', pickGallery);
  $('#dedup-profile').addEventListener('change', updateThresholdVisibility);
  $('#dedup-start').addEventListener('click', startCurrentDedupMode);
  $('#dedup-update').addEventListener('click', updateCurrentIndex);
  $('#dedup-cancel').addEventListener('click', cancelActiveJob);
  $('#dedup-result-filter').addEventListener('change', applyResultView);
  $('#dedup-result-sort').addEventListener('change', applyResultView);
  $('#dedup-prev-page').addEventListener('click', () => changePage(-1));
  $('#dedup-next-page').addEventListener('click', () => changePage(1));
  $('#dedup-manage-indexes').addEventListener('click', openIndexManager);
  $('#dedup-index-close').addEventListener('click', closeIndexManager);
  $('#dedup-index-close-footer').addEventListener('click', closeIndexManager);
  $('#dedup-delete-all-indexes').addEventListener('click', deleteAllIndexes);
  $('#dedup-mode-single').addEventListener('click', () => setDedupMode('single'));
  $('#dedup-mode-batch').addEventListener('click', () => setDedupMode('batch'));

  listen('dedup-index-progress', event => {
    const payload = event.payload;
    if (payload.job_id !== dedupState.activeJobId) return;
    showProgress(payload.current, payload.total, payload.current_file || payload.message);
  });

  updateThresholdVisibility();
  setDedupMode('single');
}

function initializeBatchController() {
  batchController = createDedupBatchController({
    getGalleryDirectories: () => dedupState.galleryDirectories,
    getThreshold: currentThreshold,
    ensureIndexes: ensureBatchIndexes,
    onJobStart: beginTask,
    onJobFinish: finishTask,
    onProgress: showProgress,
    setMessage,
    openFile: async path => {
      hideDedupWorkspace();
      await openFileCallback(path);
    },
    formatSize,
    formatNumber,
    formatDate,
  });
}

function setDedupMode(mode) {
  dedupState.mode = mode === 'batch' ? 'batch' : 'single';
  const batch = dedupState.mode === 'batch';
  $('#dedup-mode-single').classList.toggle('active', !batch);
  $('#dedup-mode-batch').classList.toggle('active', batch);
  $('#dedup-mode-single').setAttribute('aria-selected', String(!batch));
  $('#dedup-mode-batch').setAttribute('aria-selected', String(batch));
  $('#dedup-source-field').hidden = batch;
  $('#dedup-batch-scan-mode-wrap').hidden = !batch;
  $('#dedup-single-results').hidden = batch;
  $('#dedup-batch-results').hidden = !batch;
  $('#dedup-start').textContent = batch ? '开始批量查重' : '开始查重';
  $('#dedup-mode-description').textContent = batch
    ? '不需要源图片；对所有目标目录中的图片互相查重，并按关联关系分组。'
    : '选择一张源图片，在所有目标目录中查找相似图片。';
  if (batch && dedupState.visible) {
    batchController.activate().catch(error => setMessage(`读取批量任务失败：${error}`, true));
  } else {
    batchController.deactivate();
  }
}

async function startCurrentDedupMode() {
  if (dedupState.mode === 'batch') {
    try {
      await batchController.start();
    } catch (error) {
      finishTask(`批量查重失败：${error}`, true);
    }
    return;
  }
  await startDedupSearch();
}

async function showDedupWorkspace() {
  dedupState.visible = true;
  $('#main-container').style.display = 'none';
  $('#status-bar').style.display = 'none';
  $('#dedup-workspace').style.display = 'flex';
  document.title = 'imsViewer - 图片查重';
  if (!dedupState.modelChecked) await refreshModelStatus();
  if (dedupState.galleryDirectories.length) await refreshIndexStatus();
  if (dedupState.mode === 'batch') {
    await batchController.activate().catch(error => setMessage(`读取批量任务失败：${error}`, true));
  }
}

function hideDedupWorkspace() {
  dedupState.visible = false;
  batchController.deactivate();
  resetThumbnailLoading();
  $('#dedup-workspace').style.display = 'none';
  $('#main-container').style.display = 'grid';
  $('#status-bar').style.display = 'flex';
  document.title = 'imsViewer';
}

async function refreshModelStatus() {
  const label = $('#dedup-model-status');
  label.textContent = '正在加载 SSCD 模型...';
  try {
    const status = await invoke('dedup_get_model_status');
    dedupState.modelChecked = status.available;
    label.textContent = status.available
      ? `SSCD 模型已就绪 · ${status.provider}`
      : `SSCD 模型不可用 · ${status.error || '未知错误'}`;
    label.classList.toggle('error', !status.available);
  } catch (error) {
    label.textContent = `模型检查失败 · ${error}`;
    label.classList.add('error');
  }
}

async function pickSource() {
  const path = await invoke('pick_file');
  if (!path) return;
  dedupState.sourcePath = path;
  $('#dedup-source-path').value = path;
}

async function pickGallery() {
  const path = await invoke('pick_folder');
  if (!path) return;
  addGalleryDirectory(path);
  await refreshIndexStatus();
}

function clampDepth(value) {
  const depth = Number(value);
  return Number.isInteger(depth) ? Math.max(0, Math.min(100, depth)) : 0;
}

function galleryKey(path) {
  return path.replace(/\\/g, '/').replace(/\/+$/, '').toLocaleLowerCase();
}

function addGalleryDirectory(path, maxDepth = null) {
  const key = galleryKey(path);
  const existing = dedupState.galleryDirectories.find(directory => galleryKey(directory.path) === key);
  if (existing) {
    if (maxDepth !== null) existing.maxDepth = clampDepth(maxDepth);
    renderGalleryDirectories();
    return existing;
  }
  const directory = { path, maxDepth: clampDepth(maxDepth ?? 0), status: '索引状态待检查' };
  dedupState.galleryDirectories.push(directory);
  renderGalleryDirectories();
  return directory;
}

function renderGalleryDirectories() {
  const container = $('#dedup-gallery-list');
  container.innerHTML = '';
  if (!dedupState.galleryDirectories.length) {
    const empty = document.createElement('span');
    empty.className = 'dedup-gallery-empty';
    empty.textContent = '尚未添加目标目录';
    container.appendChild(empty);
    return;
  }

  dedupState.galleryDirectories.forEach((directory, index) => {
    const row = document.createElement('div');
    row.className = 'dedup-gallery-row';

    const path = document.createElement('input');
    path.className = 'path-input';
    path.type = 'text';
    path.readOnly = true;
    path.value = directory.path;
    path.title = directory.path;

    const depthLabel = document.createElement('label');
    depthLabel.className = 'dedup-gallery-depth-label';
    depthLabel.textContent = '深度';
    const depth = document.createElement('input');
    depth.className = 'depth-input dedup-gallery-depth';
    depth.type = 'number';
    depth.min = '0';
    depth.max = '100';
    depth.value = String(directory.maxDepth);
    depth.title = '0=遍历全部子目录；1=一层子目录';
    depth.disabled = dedupState.busy;
    depth.addEventListener('change', () => {
      directory.maxDepth = clampDepth(depth.value);
      depth.value = String(directory.maxDepth);
      directory.status = '深度已修改，需更新索引';
      renderGalleryDirectories();
      updateIndexStatusSummary();
    });
    depthLabel.appendChild(depth);

    const status = document.createElement('span');
    status.className = 'dedup-gallery-status';
    status.textContent = directory.status;
    status.title = directory.status;

    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'btn-secondary dedup-gallery-remove';
    remove.textContent = '移除';
    remove.disabled = dedupState.busy;
    remove.addEventListener('click', () => {
      dedupState.galleryDirectories.splice(index, 1);
      renderGalleryDirectories();
      updateIndexStatusSummary();
    });

    row.append(path, depthLabel, status, remove);
    container.appendChild(row);
  });
}

function updateThresholdVisibility() {
  const custom = $('#dedup-profile').value === 'custom';
  $('#dedup-custom-threshold-wrap').style.display = custom ? 'flex' : 'none';
}

function currentThreshold() {
  const profile = $('#dedup-profile').value;
  const value = profile === 'custom' ? Number($('#dedup-threshold').value) : Number(profile);
  return Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0.75));
}

function currentIndexConfig(directory) {
  const workerValue = Number($('#dedup-worker-count').value);
  return {
    root_path: directory.path,
    recursive: true,
    max_depth: directory.maxDepth,
    multi_threaded: $('#dedup-multi-threaded').checked,
    worker_count: Number.isInteger(workerValue) && workerValue > 0 ? workerValue : null,
  };
}

async function startDedupSearch() {
  if (!validateInputs(true)) return;
  try {
    for (const directory of dedupState.galleryDirectories) {
      await runIndexUpdate(directory);
    }
    await refreshIndexStatus();
    if (!dedupState.busy && dedupState.activeJobId === null) {
      await runSearch();
    }
  } catch (error) {
    finishTask(`查重失败：${error}`, true);
  }
}

async function updateCurrentIndex() {
  if (!validateInputs(false)) return;
  try {
    for (const directory of dedupState.galleryDirectories) {
      await runIndexUpdate(directory);
    }
    await refreshIndexStatus();
    finishTask('索引更新完成');
  } catch (error) {
    finishTask(`索引更新失败：${error}`, true);
  }
}

async function ensureBatchIndexes() {
  if (!validateInputs(false)) throw new Error('请先添加至少一个目标图库目录');
  const summaries = [];
  for (const directory of dedupState.galleryDirectories) {
    summaries.push(await runIndexUpdate(directory));
  }
  await refreshIndexStatus();
  return summaries;
}

function validateInputs(requireSource) {
  if (requireSource && !dedupState.sourcePath) {
    setMessage('请先选择源图片', true);
    return false;
  }
  if (!dedupState.galleryDirectories.length) {
    setMessage('请先添加至少一个目标图库目录', true);
    return false;
  }
  return true;
}

async function runIndexUpdate(directory, rebuild = false) {
  const jobId = crypto.randomUUID();
  beginTask(jobId, `${rebuild ? '正在重新建立' : '正在增量更新'}：${directory.path}`);
  try {
    const summary = await invoke(rebuild ? 'dedup_rebuild_index' : 'dedup_update_index', {
      jobId,
      config: currentIndexConfig(directory),
    });
    if (dedupState.activeJobId !== jobId) throw new Error('任务已取消');
    dedupState.activeJobId = null;
    dedupState.busy = false;
    setBusyControls(false);
    directory.status = `已索引 ${formatNumber(summary.final_count)} 张 · 深度 ${summary.max_depth} · ${formatSize(summary.total_bytes)}`;
    renderGalleryDirectories();
    updateIndexStatusSummary();
    showProgress(summary.final_count, summary.final_count, rebuild ? '索引重建完成' : '索引更新完成');
    return summary;
  } catch (error) {
    if (dedupState.activeJobId === jobId) {
      dedupState.activeJobId = null;
      dedupState.busy = false;
      setBusyControls(false);
    }
    throw error;
  }
}

async function runSearch() {
  const started = performance.now();
  const mergedResults = new Map();
  let totalIndexed = 0;
  try {
    for (let index = 0; index < dedupState.galleryDirectories.length; index += 1) {
      const directory = dedupState.galleryDirectories[index];
      const jobId = crypto.randomUUID();
      beginTask(jobId, `正在搜索目录 ${index + 1}/${dedupState.galleryDirectories.length}：${directory.path}`);
      showProgress(index, dedupState.galleryDirectories.length, directory.path);
      const response = await invoke('dedup_search', {
        jobId,
        request: {
          source_path: dedupState.sourcePath,
          root_path: directory.path,
          threshold: currentThreshold(),
          exclude_source: true,
        },
      });
      if (dedupState.activeJobId !== jobId) return;
      totalIndexed += response.total_indexed;
      for (const result of response.results) {
        const enriched = { ...result, index_root_path: response.root_path };
        const key = galleryKey(result.filepath);
        const existing = mergedResults.get(key);
        if (!existing || enriched.confidence > existing.confidence) mergedResults.set(key, enriched);
      }
    }
    dedupState.results = Array.from(mergedResults.values());
    dedupState.page = 0;
    applyResultView();
    finishTask(`查重完成：搜索 ${dedupState.galleryDirectories.length} 个目录、${formatNumber(totalIndexed)} 条索引，找到 ${formatNumber(dedupState.results.length)} 张，用时 ${formatDuration(performance.now() - started)}`);
  } catch (error) {
    finishTask(`查重失败：${error}`, true);
  }
}

function beginTask(jobId, message) {
  resetThumbnailLoading();
  dedupState.activeJobId = jobId;
  dedupState.busy = true;
  setBusyControls(true);
  $('#dedup-progress').style.display = 'block';
  setMessage(message, false);
}

function finishTask(message, isError = false) {
  dedupState.activeJobId = null;
  dedupState.busy = false;
  setBusyControls(false);
  setMessage(message, isError);
}

function setBusyControls(busy) {
  $('#dedup-start').disabled = busy;
  $('#dedup-update').disabled = busy;
  $('#dedup-pick-source').disabled = busy;
  $('#dedup-pick-gallery').disabled = busy;
  $('#dedup-manage-indexes').disabled = busy;
  $('#dedup-mode-single').disabled = busy;
  $('#dedup-mode-batch').disabled = busy;
  $('#dedup-batch-scan-mode').disabled = busy;
  $('#dedup-cancel').disabled = !busy;
  document.querySelectorAll('.dedup-gallery-depth, .dedup-gallery-remove').forEach(element => {
    element.disabled = busy;
  });
}

async function cancelActiveJob() {
  if (dedupState.mode === 'batch' && await batchController.cancelActive()) return;
  if (!dedupState.activeJobId) return;
  const jobId = dedupState.activeJobId;
  await invoke('dedup_cancel_job', { jobId });
  setMessage('正在取消任务...', false);
}

function showProgress(current, total, message) {
  $('#dedup-progress').style.display = 'block';
  const percent = total > 0 ? Math.min(100, Math.round(current * 100 / total)) : 10;
  $('#dedup-progress-fill').style.width = `${percent}%`;
  $('#dedup-progress-text').textContent = total > 0
    ? `${percent}% · ${formatNumber(current)} / ${formatNumber(total)} · ${message || ''}`
    : message || '处理中...';
}

function setMessage(message, isError) {
  $('#dedup-progress').style.display = 'block';
  const element = $('#dedup-progress-text');
  element.textContent = message;
  element.classList.toggle('error', Boolean(isError));
}

async function refreshIndexStatus() {
  await Promise.all(dedupState.galleryDirectories.map(async directory => {
    try {
      const status = await invoke('dedup_get_index_status', { rootPath: directory.path });
      directory.indexStatus = status;
      directory.status = status.exists
        ? `已索引 ${formatNumber(status.item_count)} 张 · 深度 ${status.max_depth}${status.max_depth !== directory.maxDepth ? `，需更新为 ${directory.maxDepth}` : ''}`
        : '尚未建立索引';
    } catch (error) {
      directory.indexStatus = null;
      directory.status = `状态读取失败：${error}`;
    }
  }));
  renderGalleryDirectories();
  updateIndexStatusSummary();
}

function updateIndexStatusSummary() {
  if (!dedupState.galleryDirectories.length) {
    $('#dedup-index-status').textContent = '尚未添加目标目录';
    return;
  }
  const statuses = dedupState.galleryDirectories.map(directory => directory.indexStatus).filter(Boolean);
  const ready = statuses.filter(status => status.exists);
  const itemCount = ready.reduce((sum, status) => sum + Number(status.item_count || 0), 0);
  const totalBytes = ready.reduce((sum, status) => sum + Number(status.total_bytes || 0), 0);
  $('#dedup-index-status').textContent = `${dedupState.galleryDirectories.length} 个目录 · ${ready.length} 套索引 · ${formatNumber(itemCount)} 张 · ${formatSize(totalBytes)}`;
}

function applyResultView() {
  const filter = $('#dedup-result-filter').value;
  const sort = $('#dedup-result-sort').value;
  dedupState.filteredResults = dedupState.results.filter(result => {
    if (filter === 'exact') return result.exact;
    if (filter === 'similar') return !result.exact && !result.template_risk;
    if (filter === 'review') return result.template_risk;
    return true;
  });
  dedupState.filteredResults.sort((left, right) => {
    if (sort === 'filename') return left.filename.localeCompare(right.filename, 'zh-CN', { numeric: true });
    if (sort === 'modified') return right.modified_ms - left.modified_ms || left.filepath.localeCompare(right.filepath);
    if (sort === 'path') return left.filepath.localeCompare(right.filepath, 'zh-CN', { numeric: true });
    return right.confidence - left.confidence || left.filepath.localeCompare(right.filepath);
  });
  const maxPage = Math.max(0, Math.ceil(dedupState.filteredResults.length / PAGE_SIZE) - 1);
  dedupState.page = Math.min(dedupState.page, maxPage);
  renderResultPage();
}

function changePage(delta) {
  const maxPage = Math.max(0, Math.ceil(dedupState.filteredResults.length / PAGE_SIZE) - 1);
  dedupState.page = Math.max(0, Math.min(maxPage, dedupState.page + delta));
  renderResultPage();
}

function renderResultPage() {
  resetThumbnailLoading();
  const generation = dedupState.thumbnailGeneration;
  const body = $('#dedup-results-body');
  body.innerHTML = '';

  const start = dedupState.page * PAGE_SIZE;
  const pageItems = dedupState.filteredResults.slice(start, start + PAGE_SIZE);
  $('#dedup-result-count').textContent = `查找结果：${formatNumber(dedupState.filteredResults.length)}`;
  const totalPages = Math.max(1, Math.ceil(dedupState.filteredResults.length / PAGE_SIZE));
  $('#dedup-page-label').textContent = `第 ${dedupState.page + 1} / ${totalPages} 页`;
  $('#dedup-prev-page').disabled = dedupState.page === 0;
  $('#dedup-next-page').disabled = dedupState.page + 1 >= totalPages;

  if (pageItems.length === 0) {
    const row = body.insertRow();
    row.className = 'dedup-empty-row';
    const cell = row.insertCell();
    cell.colSpan = 10;
    cell.textContent = dedupState.results.length ? '当前筛选条件没有结果' : '尚无查重结果';
    return;
  }

  for (const result of pageItems) {
    const row = body.insertRow();
    row.title = result.filepath;
    row.addEventListener('dblclick', () => openResult(result));

    const previewCell = row.insertCell();
    const placeholder = document.createElement('div');
    placeholder.className = 'dedup-thumb-placeholder';
    placeholder.textContent = '后台加载';
    placeholder.dataset.itemId = String(result.item_id);
    placeholder.dataset.rootPath = result.index_root_path;
    placeholder.dataset.generation = String(generation);
    previewCell.appendChild(placeholder);

    row.insertCell().textContent = `${result.confidence.toFixed(1)}%`;
    const typeCell = row.insertCell();
    const badge = document.createElement('span');
    badge.className = `dedup-match-badge${result.exact ? ' exact' : result.template_risk ? ' review' : ''}`;
    badge.textContent = result.match_type;
    badge.title = `SSCD ${result.raw_similarity.toFixed(4)} · pHash 距离 ${result.phash_distance}`;
    typeCell.appendChild(badge);
    row.insertCell().textContent = result.filename;
    row.insertCell().textContent = result.kind === 'ims_cover' ? 'IMS 封面' : '普通图片';
    row.insertCell().textContent = result.image_format;
    row.insertCell().textContent = `${result.width}×${result.height}`;
    row.insertCell().textContent = formatDate(result.modified_ms);
    row.insertCell().textContent = result.filepath;

    const actionCell = row.insertCell();
    const actions = document.createElement('div');
    actions.className = 'dedup-row-actions';
    actions.append(
      actionButton('打开', () => openResult(result)),
      actionButton('定位', () => invoke('dedup_reveal_file', { path: result.filepath })),
      actionButton('复制', () => navigator.clipboard.writeText(result.filepath)),
    );
    if (!result.exact) actions.append(actionButton('复核', () => verifyResult(result)));
    actionCell.appendChild(actions);
  }

  thumbnailObserver = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      thumbnailObserver.unobserve(entry.target);
      enqueueThumbnail(entry.target, generation);
    }
  }, { root: $('.dedup-results-wrap'), rootMargin: '120px' });
  body.querySelectorAll('.dedup-thumb-placeholder').forEach(element => thumbnailObserver.observe(element));
}

function resetThumbnailLoading() {
  dedupState.thumbnailGeneration += 1;
  dedupState.thumbnailQueue = [];
  thumbnailObserver?.disconnect();
}

function actionButton(label, handler) {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = label;
  button.addEventListener('click', event => {
    event.stopPropagation();
    Promise.resolve(handler()).catch(error => setMessage(String(error), true));
  });
  return button;
}

async function openResult(result) {
  hideDedupWorkspace();
  await openFileCallback(result.filepath);
}

async function verifyResult(result) {
  setMessage(`正在后台复核 ${result.filename} 的局部特征和文字内容...`, false);
  const verification = await invoke('dedup_verify_result', {
    request: {
      source_path: dedupState.sourcePath,
      candidate_path: result.filepath,
      raw_similarity: result.raw_similarity,
    },
  });
  result.template_risk = verification.template_risk;
  result.match_type = verification.template_risk ? '待复核' : verification.local_verified ? '局部匹配通过' : result.match_type;
  applyResultView();
  const sourceText = verification.source_text.trim().replace(/\s+/g, ' ').slice(0, 80);
  const candidateText = verification.candidate_text.trim().replace(/\s+/g, ' ').slice(0, 80);
  setMessage(
    `${verification.verdict} · 局部匹配 ${(verification.local_match_score * 100).toFixed(1)}% · OCR ${(verification.ocr_similarity * 100).toFixed(1)}%${sourceText || candidateText ? ` · 源文字「${sourceText || '--'}」· 目标文字「${candidateText || '--'}」` : ''}`,
    verification.template_risk,
  );
}

function enqueueThumbnail(element, generation) {
  dedupState.thumbnailQueue.push({
    element,
    generation,
    itemId: Number(element.dataset.itemId),
    rootPath: element.dataset.rootPath,
  });
  pumpThumbnailQueue();
}

function pumpThumbnailQueue() {
  while (dedupState.thumbnailActive < THUMBNAIL_CONCURRENCY && dedupState.thumbnailQueue.length > 0) {
    const request = dedupState.thumbnailQueue.shift();
    dedupState.thumbnailActive += 1;
    loadThumbnail(request).finally(() => {
      dedupState.thumbnailActive -= 1;
      pumpThumbnailQueue();
    });
  }
}

async function loadThumbnail({ element, generation, itemId, rootPath }) {
  try {
    const payload = await invoke('dedup_get_result_thumbnail', {
      rootPath,
      itemId,
    });
    if (generation !== dedupState.thumbnailGeneration || !element.isConnected) return;
    const image = document.createElement('img');
    image.className = 'dedup-result-thumb';
    image.alt = '';
    image.src = `data:${payload.mime};base64,${payload.bytes_base64}`;
    element.replaceWith(image);
  } catch {
    if (generation === dedupState.thumbnailGeneration && element.isConnected) element.textContent = '无法预览';
  }
}

async function openIndexManager() {
  $('#dedup-index-modal').style.display = 'flex';
  await refreshIndexManager();
}

function closeIndexManager() {
  $('#dedup-index-modal').style.display = 'none';
}

async function refreshIndexManager() {
  const body = $('#dedup-index-list');
  body.innerHTML = '<tr class="dedup-empty-row"><td colspan="7">正在加载...</td></tr>';
  try {
    const indexes = await invoke('dedup_list_indexes');
    body.innerHTML = '';
    if (!indexes.length) {
      body.innerHTML = '<tr class="dedup-empty-row"><td colspan="7">尚未建立任何图库索引</td></tr>';
      return;
    }
    for (const index of indexes) {
      const row = body.insertRow();
      row.insertCell().textContent = index.root_path;
      row.insertCell().textContent = index.max_depth === 0 ? '全部' : String(index.max_depth);
      row.insertCell().textContent = formatNumber(index.item_count);
      row.insertCell().textContent = formatSize(index.total_bytes);
      row.insertCell().textContent = formatDate(index.last_updated_ms);
      row.insertCell().textContent = index.state;
      const actions = document.createElement('div');
      actions.className = 'dedup-row-actions';
      actions.append(
        actionButton('更新', async () => {
          closeIndexManager();
          const directory = addGalleryDirectory(index.root_path, index.max_depth);
          await runIndexUpdate(directory);
          await refreshIndexStatus();
          finishTask('索引更新完成');
        }),
        actionButton('重建', () => rebuildOneIndex(index)),
        actionButton('删除', () => deleteOneIndex(index)),
      );
      row.insertCell().appendChild(actions);
    }
  } catch (error) {
    body.innerHTML = '';
    const row = body.insertRow();
    row.className = 'dedup-empty-row';
    const cell = row.insertCell();
    cell.colSpan = 7;
    cell.textContent = `加载索引失败：${error}`;
  }
}

async function rebuildOneIndex(index) {
  closeIndexManager();
  const directory = addGalleryDirectory(index.root_path, index.max_depth);
  try {
    await runIndexUpdate(directory, true);
    await refreshIndexStatus();
    finishTask('索引重建完成');
  } catch (error) {
    finishTask(`索引重建失败：${error}`, true);
  }
}

async function deleteOneIndex(index) {
  if (!confirm(`确定删除这套索引吗？\n\n图库：${index.root_path}\n图片数：${formatNumber(index.item_count)}\n预计释放：${formatSize(index.total_bytes)}\n\n不会删除任何原始图片。`)) return;
  const result = await invoke('dedup_delete_index', { indexId: index.index_id });
  setMessage(`已删除索引，释放 ${formatSize(result.released_bytes)}`, false);
  await refreshIndexManager();
  if (dedupState.galleryDirectories.some(directory => galleryKey(directory.path) === galleryKey(index.root_path))) await refreshIndexStatus();
}

async function deleteAllIndexes() {
  if (!confirm('确定删除全部图片查重索引吗？\n\n此操作不会删除任何原始图片。')) return;
  const result = await invoke('dedup_delete_all_indexes');
  setMessage(`已删除 ${result.deleted_indexes} 套索引，释放 ${formatSize(result.released_bytes)}`, false);
  await refreshIndexManager();
  if (dedupState.galleryDirectories.length) await refreshIndexStatus();
}

function formatSize(bytes) {
  if (!Number.isFinite(Number(bytes)) || Number(bytes) <= 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let value = Number(bytes);
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : value >= 100 ? 0 : 1)} ${units[unit]}`;
}

function formatNumber(value) {
  return Number(value || 0).toLocaleString('zh-CN');
}

function formatDate(milliseconds) {
  if (!milliseconds) return '--';
  return new Date(Number(milliseconds)).toLocaleString('zh-CN');
}

function formatDuration(milliseconds) {
  if (milliseconds < 1000) return `${milliseconds} ms`;
  return `${(milliseconds / 1000).toFixed(2)} 秒`;
}
