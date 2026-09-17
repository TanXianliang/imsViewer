export const BATCH_CONVERT_SETTINGS_KEY = 'imsviewer:batch-convert-settings';
export const BATCH_CONVERT_SETTINGS_VERSION = 1;

export function createDefaultBatchConvertSettings() {
  return {
    version: BATCH_CONVERT_SETTINGS_VERSION,
    folders: [{ path: '', depth: 0 }],
    output_dir: '',
    delete_source: false,
    multi_threaded: false,
    preserve_structure: true,
    merge_existing_ims: false,
    sub_images_to_jpeg: false,
  };
}

function normalizeDepth(value) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed)) return 0;
  return Math.min(100, Math.max(0, parsed));
}

function normalizeBoolean(value, fallback) {
  return typeof value === 'boolean' ? value : fallback;
}

export function normalizeBatchConvertSettings(candidate) {
  const defaults = createDefaultBatchConvertSettings();
  if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) {
    return defaults;
  }
  if (candidate.version !== BATCH_CONVERT_SETTINGS_VERSION) {
    return defaults;
  }

  const folders = Array.isArray(candidate.folders)
    ? candidate.folders
      .filter(folder => folder && typeof folder === 'object' && typeof folder.path === 'string')
      .map(folder => ({
        path: folder.path,
        depth: normalizeDepth(folder.depth),
      }))
    : [];

  return {
    version: BATCH_CONVERT_SETTINGS_VERSION,
    folders: folders.length > 0 ? folders : defaults.folders,
    output_dir: typeof candidate.output_dir === 'string' ? candidate.output_dir : defaults.output_dir,
    delete_source: normalizeBoolean(candidate.delete_source, defaults.delete_source),
    multi_threaded: normalizeBoolean(candidate.multi_threaded, defaults.multi_threaded),
    preserve_structure: normalizeBoolean(candidate.preserve_structure, defaults.preserve_structure),
    merge_existing_ims: normalizeBoolean(candidate.merge_existing_ims, defaults.merge_existing_ims),
    sub_images_to_jpeg: normalizeBoolean(candidate.sub_images_to_jpeg, defaults.sub_images_to_jpeg),
  };
}

export function loadBatchConvertSettings(storage) {
  try {
    const raw = storage?.getItem(BATCH_CONVERT_SETTINGS_KEY);
    if (!raw) return createDefaultBatchConvertSettings();
    return normalizeBatchConvertSettings(JSON.parse(raw));
  } catch {
    return createDefaultBatchConvertSettings();
  }
}

export function saveBatchConvertSettings(storage, settings) {
  try {
    const normalized = normalizeBatchConvertSettings(settings);
    storage?.setItem(BATCH_CONVERT_SETTINGS_KEY, JSON.stringify(normalized));
    return Boolean(storage);
  } catch {
    return false;
  }
}
