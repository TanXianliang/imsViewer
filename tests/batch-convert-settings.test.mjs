import test from 'node:test';
import assert from 'node:assert/strict';

import {
  BATCH_CONVERT_SETTINGS_KEY,
  createDefaultBatchConvertSettings,
  loadBatchConvertSettings,
  normalizeBatchConvertSettings,
  saveBatchConvertSettings,
} from '../src/batch-convert-settings.js';

function createMemoryStorage(initialValue = null) {
  const values = new Map();
  if (initialValue !== null) values.set(BATCH_CONVERT_SETTINGS_KEY, initialValue);
  return {
    getItem(key) {
      return values.has(key) ? values.get(key) : null;
    },
    setItem(key, value) {
      values.set(key, value);
    },
  };
}

test('missing settings use the current batch-convert defaults', () => {
  const actual = loadBatchConvertSettings(createMemoryStorage());

  assert.deepEqual(actual, createDefaultBatchConvertSettings());
  assert.deepEqual(actual.folders, [{ path: '', depth: 0 }]);
  assert.equal(actual.preserve_structure, true);
  assert.equal(actual.delete_source, false);
});

test('settings round-trip through storage without losing any supported field', () => {
  const storage = createMemoryStorage();
  const settings = {
    version: 1,
    folders: [
      { path: 'D:\\images', depth: 4 },
      { path: 'E:\\more images', depth: 0 },
    ],
    output_dir: 'F:\\converted',
    delete_source: true,
    multi_threaded: true,
    preserve_structure: false,
    merge_existing_ims: true,
    sub_images_to_jpeg: true,
  };

  assert.equal(saveBatchConvertSettings(storage, settings), true);
  assert.deepEqual(loadBatchConvertSettings(storage), settings);
});

test('malformed JSON and unsupported versions fall back to defaults', () => {
  assert.deepEqual(
    loadBatchConvertSettings(createMemoryStorage('{not-json')),
    createDefaultBatchConvertSettings(),
  );
  assert.deepEqual(
    loadBatchConvertSettings(createMemoryStorage(JSON.stringify({ version: 99 }))),
    createDefaultBatchConvertSettings(),
  );
});

test('normalization accepts only known types and clamps folder depth', () => {
  const actual = normalizeBatchConvertSettings({
    version: 1,
    folders: [
      { path: 'C:\\low', depth: -9 },
      { path: 'C:\\high', depth: 500 },
      { path: 'C:\\string', depth: '7' },
      { path: 123, depth: 2 },
      null,
    ],
    output_dir: 42,
    delete_source: 'yes',
    multi_threaded: true,
    preserve_structure: null,
    merge_existing_ims: false,
    sub_images_to_jpeg: true,
  });

  assert.deepEqual(actual, {
    version: 1,
    folders: [
      { path: 'C:\\low', depth: 0 },
      { path: 'C:\\high', depth: 100 },
      { path: 'C:\\string', depth: 7 },
    ],
    output_dir: '',
    delete_source: false,
    multi_threaded: true,
    preserve_structure: true,
    merge_existing_ims: false,
    sub_images_to_jpeg: true,
  });
});

test('storage failures do not escape into the batch-convert UI', () => {
  const throwingStorage = {
    getItem() {
      throw new Error('read disabled');
    },
    setItem() {
      throw new Error('write disabled');
    },
  };

  assert.deepEqual(
    loadBatchConvertSettings(throwingStorage),
    createDefaultBatchConvertSettings(),
  );
  assert.equal(
    saveBatchConvertSettings(throwingStorage, createDefaultBatchConvertSettings()),
    false,
  );
});
