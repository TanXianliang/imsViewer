import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const GROUP_PAGE_SIZE = 30;
const MEMBER_PAGE_SIZE = 200;
const THUMBNAIL_CONCURRENCY = 3;

const statusLabels = {
  preparing: '准备中',
  running: '扫描中',
  finalizing: '整理分组',
  completed: '已完成',
  cancelled: '已取消',
  interrupted: '已中断',
  failed: '失败',
  stale: '结果已过期',
};

export function createDedupBatchController({
  getGalleryDirectories,
  getThreshold,
  ensureIndexes,
  onJobStart,
  onJobFinish,
  onProgress,
  setMessage,
  openFile,
  formatSize,
  formatNumber,
  formatDate,
}) {
  const state = {
    activeRunId: null,
    selectedRunId: null,
    groupPage: 0,
    groupTotal: 0,
    autoDeleteThresholdPercent: 99,
    thumbnailGeneration: 0,
    thumbnailQueue: [],
    thumbnailActive: 0,
    active: false,
  };

  listen('dedup-batch-progress', event => {
    const payload = event.payload;
    if (payload.run_id !== state.activeRunId) return;
    const phase = phaseLabel(payload.phase);
    const suffix = `候选 ${formatNumber(payload.candidate_pairs)} · 已连接 ${formatNumber(payload.accepted_pairs)}`;
    onProgress(payload.processed_pairs, payload.total_pairs, `${phase} · ${suffix}`);
  }).catch(error => setMessage(`监听批量任务进度失败：${error}`, true));

  return {
    activate,
    deactivate,
    start,
    cancelActive,
    refreshRuns,
  };

  async function activate() {
    state.active = true;
    await refreshRuns();
  }

  function deactivate() {
    state.active = false;
    resetThumbnailQueue();
  }

  async function start() {
    const directories = getGalleryDirectories();
    if (!directories.length) throw new Error('请先添加至少一个目标图库目录');
    const summaries = await ensureIndexes();
    const mode = document.querySelector('#dedup-batch-scan-mode').value;
    if (mode === 'complete') {
      const count = summaries.reduce((total, summary) => total + Number(summary.final_count || 0), 0);
      const pairs = count * Math.max(0, count - 1) / 2;
      if (!confirm(`完整扫描预计检查 ${formatNumber(pairs)} 个图片对，可能耗时很长。任务可以取消并继续，是否开始？`)) {
        onJobFinish('已取消启动完整扫描');
        return;
      }
    }

    const runId = crypto.randomUUID();
    state.activeRunId = runId;
    onJobStart(runId, mode === 'quick' ? '正在启动快速批量查重...' : '正在启动完整批量查重...');
    try {
      const summary = await invoke('dedup_start_batch_scan', {
        jobId: runId,
        request: {
          root_paths: directories.map(directory => directory.path),
          mode,
          threshold: getThreshold(),
        },
      });
      state.selectedRunId = summary.run_id;
      await Promise.all([refreshRuns(false), loadGroupPage(summary.run_id, 0)]);
      onJobFinish(`批量查重完成：${formatNumber(summary.group_count)} 组`);
    } catch (error) {
      onJobFinish(`批量查重失败：${error}`, true);
      await refreshRuns(false).catch(() => {});
    } finally {
      state.activeRunId = null;
    }
  }

  async function cancelActive() {
    if (!state.activeRunId) return false;
    await invoke('dedup_cancel_job', { jobId: state.activeRunId });
    setMessage('正在取消批量任务；完整扫描会先保存当前检查点...', false);
    return true;
  }

  async function refreshRuns(autoSelect = true) {
    const paths = getGalleryDirectories().map(directory => directory.path);
    const runs = await invoke('dedup_list_batch_runs', {
      rootPaths: paths.length ? paths : null,
    });
    renderRuns(runs);
    if (!state.active || !autoSelect || state.selectedRunId || !runs.length) return;
    const firstWithResults = runs.find(run => run.group_count > 0);
    if (firstWithResults) {
      state.selectedRunId = firstWithResults.run_id;
      await loadGroupPage(firstWithResults.run_id, 0);
    }
  }

  function renderRuns(runs) {
    const container = document.querySelector('#dedup-batch-runs');
    container.replaceChildren();
    const heading = document.createElement('div');
    heading.className = 'dedup-batch-section-heading';
    heading.textContent = `批量任务记录（${formatNumber(runs.length)}）`;
    container.appendChild(heading);
    if (!runs.length) {
      const empty = document.createElement('div');
      empty.className = 'dedup-batch-empty';
      empty.textContent = '当前目标目录还没有批量查重记录。';
      container.appendChild(empty);
      return;
    }
    for (const run of runs) {
      const row = document.createElement('article');
      row.className = `dedup-batch-run status-${run.status}`;

      const open = document.createElement('button');
      open.type = 'button';
      open.className = 'dedup-batch-run-open';
      open.textContent = `${run.mode === 'quick' ? '快速扫描' : '完整扫描'} · ${statusLabels[run.status] || run.status}`;
      open.title = run.root_paths.join('\n');
      open.addEventListener('click', () => {
        state.selectedRunId = run.run_id;
        loadGroupPage(run.run_id, 0).catch(error => setMessage(`读取批量结果失败：${error}`, true));
      });

      const details = document.createElement('span');
      details.className = 'dedup-batch-run-details';
      details.textContent = `${formatNumber(run.group_count)} 组 · ${formatNumber(run.accepted_pairs)} 条关联 · ${formatDate(run.updated_ms)}`;
      row.append(open, details);

      if (run.resumable) {
        const resume = document.createElement('button');
        resume.type = 'button';
        resume.className = 'btn-secondary';
        resume.textContent = '继续扫描';
        resume.addEventListener('click', () => resumeRun(run.run_id));
        row.appendChild(resume);
      }

      const remove = document.createElement('button');
      remove.type = 'button';
      remove.className = 'btn-secondary';
      remove.textContent = '删除记录';
      remove.disabled = ['preparing', 'running', 'finalizing'].includes(run.status);
      remove.addEventListener('click', async () => {
        if (!confirm('确定删除这套批量查重结果吗？\n不会删除图库索引或任何原始图片。')) return;
        await invoke('dedup_delete_batch_run', { runId: run.run_id });
        if (state.selectedRunId === run.run_id) {
          state.selectedRunId = null;
          renderEmptyGroups('批量结果记录已删除。');
        }
        await refreshRuns(false);
      });
      row.appendChild(remove);
      container.appendChild(row);
    }
  }

  async function resumeRun(runId) {
    state.activeRunId = runId;
    onJobStart(runId, '正在从检查点继续完整扫描...');
    try {
      const summary = await invoke('dedup_resume_batch_scan', { runId });
      state.selectedRunId = runId;
      await Promise.all([refreshRuns(false), loadGroupPage(runId, 0)]);
      onJobFinish(`批量查重完成：${formatNumber(summary.group_count)} 组`);
    } catch (error) {
      onJobFinish(`继续扫描失败：${error}`, true);
      await refreshRuns(false).catch(() => {});
    } finally {
      state.activeRunId = null;
    }
  }

  async function loadGroupPage(runId, page) {
    resetThumbnailQueue();
    const response = await invoke('dedup_get_batch_groups', {
      runId,
      page,
      pageSize: GROUP_PAGE_SIZE,
    });
    state.groupPage = response.page;
    state.groupTotal = response.total;
    const container = document.querySelector('#dedup-batch-groups');
    container.replaceChildren();

    const toolbar = document.createElement('div');
    toolbar.className = 'dedup-batch-groups-toolbar';
    const title = document.createElement('strong');
    title.textContent = `重复图片组：${formatNumber(response.total)}`;
    const autoDeleteControls = document.createElement('div');
    autoDeleteControls.className = 'dedup-auto-delete-controls';
    const thresholdLabel = document.createElement('label');
    thresholdLabel.textContent = '相似度 >';
    const thresholdInput = document.createElement('input');
    thresholdInput.type = 'number';
    thresholdInput.min = '0';
    thresholdInput.max = '100';
    thresholdInput.step = '0.1';
    thresholdInput.value = String(state.autoDeleteThresholdPercent);
    thresholdInput.setAttribute('aria-label', '一键删除相似度阈值百分比');
    thresholdInput.addEventListener('input', () => {
      const value = Number(thresholdInput.value);
      if (Number.isFinite(value)) state.autoDeleteThresholdPercent = value;
    });
    const percent = document.createElement('span');
    percent.textContent = '%';
    const autoDelete = document.createElement('button');
    autoDelete.type = 'button';
    autoDelete.className = 'btn-danger dedup-auto-delete';
    autoDelete.textContent = '一键删除';
    autoDelete.title = '每个高相似度图片簇保留质量最佳的一张，永久删除其余图片';
    autoDelete.addEventListener('click', () => autoDeleteBatchFiles(runId, thresholdInput, autoDelete));
    thresholdLabel.append(thresholdInput, percent);
    autoDeleteControls.append(thresholdLabel, autoDelete);
    const previous = document.createElement('button');
    previous.type = 'button';
    previous.className = 'btn-secondary';
    previous.textContent = '上一页';
    previous.disabled = response.page === 0;
    previous.addEventListener('click', () => loadGroupPage(runId, response.page - 1));
    const pages = Math.max(1, Math.ceil(response.total / response.page_size));
    const label = document.createElement('span');
    label.textContent = `第 ${response.page + 1} / ${pages} 页`;
    const next = document.createElement('button');
    next.type = 'button';
    next.className = 'btn-secondary';
    next.textContent = '下一页';
    next.disabled = response.page + 1 >= pages;
    next.addEventListener('click', () => loadGroupPage(runId, response.page + 1));
    toolbar.append(title, autoDeleteControls, previous, label, next);
    container.appendChild(toolbar);

    if (!response.items.length) {
      const empty = document.createElement('div');
      empty.className = 'dedup-batch-empty';
      empty.textContent = '没有找到达到当前严格阈值的重复图片组。';
      container.appendChild(empty);
      return;
    }
    response.items.forEach(group => container.appendChild(renderGroupCard(runId, group)));
  }

  async function autoDeleteBatchFiles(runId, thresholdInput, button) {
    const thresholdPercent = Number(thresholdInput.value);
    if (!Number.isFinite(thresholdPercent) || thresholdPercent < 0 || thresholdPercent > 100) {
      setMessage('一键删除相似度必须在 0% 到 100% 之间。', true);
      thresholdInput.focus();
      return;
    }
    state.autoDeleteThresholdPercent = thresholdPercent;
    button.disabled = true;
    button.textContent = '正在统计...';
    try {
      const threshold = thresholdPercent / 100;
      const preview = await invoke('dedup_preview_batch_auto_delete', { runId, threshold });
      if (!preview.candidate_count) {
        setMessage(`没有相似度高于 ${thresholdPercent}% 且可安全删除的重复图片。`, false);
        return;
      }
      const confirmed = confirm(
        `将永久删除 ${formatNumber(preview.candidate_count)} 张图片，预计释放 ${formatSize(preview.candidate_bytes)}。\n\n` +
        `仅处理相似度严格高于 ${thresholdPercent}% 的关联；每个高相似度图片簇至少保留质量最佳的一张。\n\n` +
        '此操作不可撤销，是否继续？',
      );
      if (!confirmed) return;
      button.textContent = '删除中...';
      const result = await invoke('dedup_auto_delete_batch_files', { runId, threshold });
      await Promise.all([refreshRuns(false), loadGroupPage(runId, 0)]);
      const skipped = result.missing_count
        ? `，另有 ${formatNumber(result.missing_count)} 张文件已不存在`
        : '';
      const failed = result.failed_count
        ? `，${formatNumber(result.failed_count)} 张删除失败`
        : '';
      setMessage(
        `一键删除完成：永久删除 ${formatNumber(result.deleted_count)} 张，释放 ${formatSize(result.released_bytes)}${skipped}${failed}。请执行“仅更新索引”清理索引记录。`,
        result.failed_count > 0,
      );
      if (result.failures.length) {
        console.warn('一键删除部分文件失败', result.failures);
      }
    } catch (error) {
      setMessage(`一键删除失败：${error}`, true);
    } finally {
      if (button.isConnected) {
        button.disabled = false;
        button.textContent = '一键删除';
      }
    }
  }

  function renderGroupCard(runId, group) {
    const card = document.createElement('article');
    card.className = 'dedup-group-card';
    const toggle = document.createElement('button');
    toggle.type = 'button';
    toggle.className = 'dedup-group-toggle';
    toggle.setAttribute('aria-expanded', 'false');
    const similarity = group.min_similarity === group.max_similarity
      ? `${(group.max_similarity * 100).toFixed(1)}%`
      : `${(group.min_similarity * 100).toFixed(1)}%–${(group.max_similarity * 100).toFixed(1)}%`;
    toggle.textContent = `${formatNumber(group.member_count)} 张 · 相似度 ${similarity} · 预计可释放 ${formatSize(group.reclaimable_bytes)}`;
    const members = document.createElement('div');
    members.className = 'dedup-group-members';
    members.hidden = true;
    toggle.addEventListener('click', async () => {
      const opening = members.hidden;
      members.hidden = !opening;
      toggle.setAttribute('aria-expanded', String(opening));
      if (!opening || members.dataset.loaded === 'true') return;
      members.textContent = '正在后台加载组成员...';
      try {
        const response = await invoke('dedup_get_batch_group_members', {
          runId,
          groupId: group.group_id,
          page: 0,
          pageSize: MEMBER_PAGE_SIZE,
        });
        members.replaceChildren(...response.items.map(renderMember));
        members.dataset.loaded = 'true';
        observeThumbnails(members, state.thumbnailGeneration);
      } catch (error) {
        members.textContent = `加载失败：${error}`;
      }
    });
    card.append(toggle, members);
    return card;
  }

  function renderMember(member) {
    const item = document.createElement('article');
    item.className = `dedup-group-member${member.recommended ? ' recommended' : ''}`;
    const preview = document.createElement('div');
    preview.className = 'dedup-thumb-placeholder';
    preview.textContent = '后台加载';
    preview.dataset.itemId = String(member.item_id);
    preview.dataset.rootPath = member.index_root_path;

    const info = document.createElement('div');
    info.className = 'dedup-group-member-info';
    const name = document.createElement('strong');
    name.textContent = `${member.recommended ? '建议保留 · ' : ''}${member.filename}`;
    name.title = member.filepath;
    const meta = document.createElement('span');
    const score = member.similarity == null ? '' : ` · 关联 ${(member.similarity * 100).toFixed(1)}%`;
    meta.textContent = `${member.width}×${member.height} · ${formatSize(member.file_size)}${score}`;
    const path = document.createElement('span');
    path.className = 'dedup-member-path';
    path.textContent = member.filepath;
    path.title = member.filepath;
    const actions = document.createElement('div');
    actions.className = 'dedup-member-actions';
    const open = document.createElement('button');
    open.type = 'button';
    open.className = 'btn-secondary';
    open.textContent = '打开';
    open.addEventListener('click', () => openFile(member.filepath));
    const reveal = document.createElement('button');
    reveal.type = 'button';
    reveal.className = 'btn-secondary';
    reveal.textContent = '定位';
    reveal.addEventListener('click', () => invoke('dedup_reveal_file', { path: member.filepath }));
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'btn-danger dedup-member-delete';
    remove.textContent = '删除文件';
    remove.addEventListener('click', async () => {
      const recommendation = member.recommended ? '\n该图片是当前组的建议保留项。' : '';
      if (!confirm(`确定永久删除这个图片文件吗？${recommendation}\n\n${member.filepath}\n\n此操作不可撤销。`)) return;
      remove.disabled = true;
      remove.textContent = '删除中...';
      try {
        await invoke('dedup_delete_indexed_file', {
          rootPath: member.index_root_path,
          itemId: member.item_id,
        });
        item.classList.add('deleted');
        open.disabled = true;
        reveal.disabled = true;
        remove.textContent = '已删除';
        name.textContent = `已删除 · ${member.filename}`;
        const currentPreview = item.querySelector('.dedup-result-thumb, .dedup-thumb-placeholder');
        if (currentPreview) {
          const deleted = document.createElement('div');
          deleted.className = 'dedup-thumb-placeholder';
          deleted.textContent = '文件已删除';
          currentPreview.replaceWith(deleted);
        }
        setMessage(`已删除图片：${member.filepath}。执行“仅更新索引”可清理索引记录。`, false);
      } catch (error) {
        remove.disabled = false;
        remove.textContent = '删除文件';
        setMessage(`删除图片失败：${error}`, true);
      }
    });
    actions.append(open, reveal, remove);
    info.append(name, meta, path, actions);
    item.append(preview, info);
    return item;
  }

  function observeThumbnails(container, generation) {
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        observer.unobserve(entry.target);
        state.thumbnailQueue.push({ element: entry.target, generation });
        pumpThumbnails();
      }
    }, { rootMargin: '160px' });
    container.querySelectorAll('.dedup-thumb-placeholder').forEach(element => observer.observe(element));
  }

  function pumpThumbnails() {
    while (state.thumbnailActive < THUMBNAIL_CONCURRENCY && state.thumbnailQueue.length) {
      const request = state.thumbnailQueue.shift();
      state.thumbnailActive += 1;
      loadThumbnail(request).finally(() => {
        state.thumbnailActive -= 1;
        pumpThumbnails();
      });
    }
  }

  async function loadThumbnail({ element, generation }) {
    try {
      const payload = await invoke('dedup_get_result_thumbnail', {
        rootPath: element.dataset.rootPath,
        itemId: Number(element.dataset.itemId),
      });
      if (generation !== state.thumbnailGeneration || !element.isConnected) return;
      const image = document.createElement('img');
      image.className = 'dedup-result-thumb';
      image.alt = '';
      image.src = `data:${payload.mime};base64,${payload.bytes_base64}`;
      element.replaceWith(image);
    } catch (error) {
      if (generation === state.thumbnailGeneration && element.isConnected) {
        element.textContent = '无法预览';
        element.title = String(error);
      }
      console.warn('批量结果缩略图加载失败', error);
    }
  }

  function resetThumbnailQueue() {
    state.thumbnailGeneration += 1;
    state.thumbnailQueue = [];
  }

  function renderEmptyGroups(message) {
    const container = document.querySelector('#dedup-batch-groups');
    const empty = document.createElement('div');
    empty.className = 'dedup-batch-empty';
    empty.textContent = message;
    container.replaceChildren(empty);
  }
}

function phaseLabel(phase) {
  return ({
    preparing: '读取索引',
    ann_build: '建立候选索引',
    quick: '快速扫描',
    complete: '完整扫描',
    grouping: '整理分组',
    finished: '任务完成',
  })[phase] || phase;
}
