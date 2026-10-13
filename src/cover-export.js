export function createCoverExportTool({ invoke, listen, root = document, showAlert = alert }) {
  const $ = selector => root.querySelector(selector);
  const modal = $('#cover-export-modal');
  const startButton = $('#btn-start-cover-export');
  const progressFill = $('#cover-export-progress-fill');
  const progressText = $('#cover-export-progress-text');
  let running = false;
  let unlisten = null;

  function setRunning(value) {
    running = value;
    startButton.disabled = value;
    startButton.textContent = value ? '导出中...' : '开始导出';
    modal.querySelectorAll('input, .btn-browse').forEach(control => {
      control.disabled = value;
    });
  }

  function stopListening() {
    if (unlisten) unlisten();
    unlisten = null;
  }

  async function pickDirectory(inputSelector) {
    try {
      const directory = await invoke('pick_folder');
      if (directory) $(inputSelector).value = directory;
    } catch (error) {
      showAlert('选择目录失败: ' + error);
    }
  }

  async function start() {
    if (running) return;
    const inputDir = $('#cover-export-dir-input').value.trim();
    const outputDir = $('#cover-export-output-dir-input').value.trim();
    if (!inputDir) { showAlert('请选择源目录'); return; }
    if (!outputDir) { showAlert('请选择输出目录'); return; }
    const config = {
      input_dir: inputDir,
      output_dir: outputDir,
      depth: Math.min(100, Math.max(0, parseInt($('#cover-export-depth-input').value, 10) || 0)),
      preserve_structure: $('#chk-cover-export-preserve-structure').checked,
      multi_threaded: $('#chk-cover-export-multi').checked,
    };

    setRunning(true);
    progressFill.style.width = '0%';
    progressText.textContent = '正在扫描 IMS 文件...';
    try {
      stopListening();
      unlisten = await listen('batch-cover-export-progress', ({ payload }) => {
        const percent = payload.total > 0 ? Math.round(payload.current / payload.total * 100) : 0;
        progressFill.style.width = percent + '%';
        progressText.textContent = `[${payload.current}/${payload.total}] ${payload.message}`;
        if (payload.status === 'completed' || payload.status === 'error') {
          setRunning(false);
          stopListening();
        }
      });
      await invoke('start_batch_export_covers', { config });
    } catch (error) {
      setRunning(false);
      stopListening();
      progressText.textContent = '启动失败: ' + error;
      showAlert('启动封面导出失败: ' + error);
    }
  }

  $('#btn-browse-cover-export').onclick = () => pickDirectory('#cover-export-dir-input');
  $('#btn-browse-cover-export-output').onclick = () => pickDirectory('#cover-export-output-dir-input');
  $('#btn-cover-export-close').onclick = () => { modal.style.display = 'none'; };
  $('#btn-close-cover-export').onclick = () => { modal.style.display = 'none'; };
  startButton.onclick = start;

  return {
    open() {
      modal.style.display = 'flex';
    },
  };
}
