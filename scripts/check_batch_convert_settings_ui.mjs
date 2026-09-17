import { readFileSync } from 'node:fs';

const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const appScript = readFileSync(new URL('../src/app.js', import.meta.url), 'utf8');
const settingsScript = readFileSync(new URL('../src/batch-convert-settings.js', import.meta.url), 'utf8');

for (const id of [
  'folder-list',
  'output-dir-input',
  'chk-delete-source',
  'chk-multi-thread',
  'chk-preserve-structure',
  'chk-merge-existing-ims',
  'chk-sub-to-jpeg',
]) {
  if (!html.includes(`id="${id}"`)) throw new Error(`missing #${id} in index.html`);
  if (!appScript.includes(`#${id}`)) throw new Error(`unused #${id} in batch settings UI`);
}

for (const field of [
  'folders',
  'output_dir',
  'delete_source',
  'multi_threaded',
  'preserve_structure',
  'merge_existing_ims',
  'sub_images_to_jpeg',
]) {
  if (!settingsScript.includes(field)) throw new Error(`missing persisted field: ${field}`);
}

for (const integration of [
  'applyBatchConvertSettings(loadBatchConvertSettings(',
  'function persistBatchConvertSettings()',
  "$('#batch-modal').oninput",
]) {
  if (!appScript.includes(integration)) throw new Error(`missing UI integration: ${integration}`);
}

console.log('batch convert settings UI contract OK');
