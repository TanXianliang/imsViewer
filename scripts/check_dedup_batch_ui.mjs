import { readFileSync } from 'node:fs';

const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const dedupScript = readFileSync(new URL('../src/dedup.js', import.meta.url), 'utf8');
const batchScript = readFileSync(new URL('../src/dedup-batch.js', import.meta.url), 'utf8');
const scripts = [dedupScript, batchScript].join('\n');

for (const id of [
  'dedup-mode-single',
  'dedup-mode-batch',
  'dedup-source-field',
  'dedup-batch-scan-mode',
  'dedup-batch-runs',
  'dedup-batch-groups',
]) {
  if (!html.includes(`id="${id}"`)) throw new Error(`missing #${id} in index.html`);
  if (!scripts.includes(`#${id}`)) throw new Error(`unused #${id} in dedup scripts`);
}

if (!batchScript.includes("invoke('dedup_delete_indexed_file'")) {
  throw new Error('missing indexed-file delete action in batch results');
}
for (const command of [
  'dedup_preview_batch_auto_delete',
  'dedup_auto_delete_batch_files',
]) {
  if (!batchScript.includes(`invoke('${command}'`)) {
    throw new Error(`missing ${command} action in batch results`);
  }
}
if (!batchScript.includes('autoDeleteThresholdPercent: 99')) {
  throw new Error('batch auto-delete threshold must default to 99%');
}
if (!batchScript.includes('observeThumbnails(members, state.thumbnailGeneration)')) {
  throw new Error('batch thumbnails must share the active result generation');
}
if (batchScript.includes('observeThumbnails(members, ++state.thumbnailGeneration)')) {
  throw new Error('expanding a group must not invalidate existing thumbnail requests');
}

console.log('dedup batch UI contract OK');
