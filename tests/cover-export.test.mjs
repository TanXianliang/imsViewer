import assert from 'node:assert/strict';
import test from 'node:test';
import { createCoverExportTool } from '../src/cover-export.js';

function createHarness() {
  const elements = new Map();
  const element = selector => {
    if (!elements.has(selector)) elements.set(selector, {
      style: {}, value: '', checked: false, disabled: false, textContent: '',
    });
    return elements.get(selector);
  };
  const controls = [
    '#cover-export-dir-input', '#cover-export-output-dir-input', '#cover-export-depth-input',
    '#chk-cover-export-preserve-structure', '#chk-cover-export-multi',
    '#btn-browse-cover-export', '#btn-browse-cover-export-output',
  ].map(element);
  element('#cover-export-modal').querySelectorAll = () => controls;
  element('#cover-export-depth-input').value = '0';
  element('#chk-cover-export-preserve-structure').checked = true;
  const calls = [];
  const alerts = [];
  let callback;
  let eventName;
  let unsubscribed = 0;
  let handler = async () => null;
  const tool = createCoverExportTool({
    root: { querySelector: element },
    showAlert: message => alerts.push(message),
    invoke: async (command, args) => {
      calls.push({ command, args });
      return handler(command, args);
    },
    listen: async (name, onEvent) => {
      eventName = name;
      callback = onEvent;
      return () => { unsubscribed++; };
    },
  });
  return {
    tool, element, calls, alerts, controls,
    setHandler(value) { handler = value; },
    start() { return element('#btn-start-cover-export').onclick(); },
    emit(payload) { callback({ payload }); },
    get eventName() { return eventName; },
    get unsubscribed() { return unsubscribed; },
  };
}

function setDirectories(harness) {
  harness.element('#cover-export-dir-input').value = ' C:\\images ';
  harness.element('#cover-export-output-dir-input').value = ' D:\\covers ';
}

test('requires both directories before invoking the export command', async () => {
  const harness = createHarness();
  await harness.start();
  assert.deepEqual(harness.alerts, ['请选择源目录']);
  harness.element('#cover-export-dir-input').value = 'C:\\images';
  await harness.start();
  assert.equal(harness.alerts.at(-1), '请选择输出目录');
  assert.equal(harness.calls.length, 0);
});

test('directory browsing keeps the previous value when the picker is cancelled', async () => {
  const harness = createHarness();
  harness.setHandler(async () => 'C:\\images');
  await harness.element('#btn-browse-cover-export').onclick();
  assert.equal(harness.element('#cover-export-dir-input').value, 'C:\\images');
  harness.setHandler(async () => null);
  await harness.element('#btn-browse-cover-export').onclick();
  assert.equal(harness.element('#cover-export-dir-input').value, 'C:\\images');
  harness.setHandler(async () => 'D:\\covers');
  await harness.element('#btn-browse-cover-export-output').onclick();
  assert.equal(harness.element('#cover-export-output-dir-input').value, 'D:\\covers');
});

test('exports the selected options and keeps progress while closed or reopened', async () => {
  const harness = createHarness();
  setDirectories(harness);
  harness.element('#cover-export-depth-input').value = '500';
  harness.element('#chk-cover-export-preserve-structure').checked = false;
  harness.element('#chk-cover-export-multi').checked = true;
  await harness.start();
  assert.equal(harness.eventName, 'batch-cover-export-progress');
  assert.deepEqual(harness.calls[0], {
    command: 'start_batch_export_covers',
    args: { config: {
      input_dir: 'C:\\images', output_dir: 'D:\\covers', depth: 100,
      preserve_structure: false, multi_threaded: true,
    } },
  });
  harness.emit({ current: 1, total: 2, message: '已导出 a.png', status: 'running' });
  harness.element('#btn-close-cover-export').onclick();
  harness.tool.open();
  assert.equal(harness.element('#cover-export-modal').style.display, 'flex');
  assert.equal(harness.element('#cover-export-progress-fill').style.width, '50%');
  assert.equal(harness.element('#btn-start-cover-export').disabled, true);
  assert.ok(harness.controls.every(control => control.disabled));
  await harness.start();
  assert.equal(harness.calls.length, 1);
  harness.emit({ current: 2, total: 2, message: '完成', status: 'completed' });
  assert.equal(harness.element('#cover-export-progress-fill').style.width, '100%');
  assert.equal(harness.element('#btn-start-cover-export').disabled, false);
  assert.ok(harness.controls.every(control => !control.disabled));
  assert.equal(harness.unsubscribed, 1);
});

test('startup failures release controls and listener so export can be retried', async () => {
  const harness = createHarness();
  setDirectories(harness);
  harness.setHandler(async () => { throw new Error('未发现 IMS 文件'); });
  await harness.start();
  assert.equal(harness.element('#btn-start-cover-export').disabled, false);
  assert.match(harness.element('#cover-export-progress-text').textContent, /未发现 IMS 文件/);
  assert.equal(harness.unsubscribed, 1);
  harness.setHandler(async () => '已启动');
  await harness.start();
  assert.equal(harness.calls.length, 2);
  harness.emit({ current: 2, total: 2, message: '成功 1 个，失败 1 个', status: 'error' });
  assert.equal(harness.element('#btn-start-cover-export').disabled, false);
  assert.match(harness.element('#cover-export-progress-text').textContent, /失败 1 个/);
  assert.equal(harness.unsubscribed, 2);
});
