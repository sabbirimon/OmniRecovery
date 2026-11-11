'use strict';

const el = id => document.getElementById(id);
const invoke = (name, args = {}) => window.__TAURI__.core.invoke(name, args);
const selections = new Map();
const number = new Intl.NumberFormat();
let busy = false;
let scanReport = null;
let activeFinding = null;
let repairReport = null;
let mediaSaving = false;

function size(bytes) {
  if (!Number.isFinite(bytes)) return 'Unavailable';
  if (bytes < 1024) return `${number.format(bytes)} B`;
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 4);
  return `${number.format(Math.round(bytes / 1024 ** unit * 10) / 10)} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][unit]}`;
}

function words(value) { return typeof value === 'string' ? value.replaceAll('_', ' ') : 'Unavailable'; }
function text(tag, value, className) {
  const node = document.createElement(tag);
  node.textContent = value;
  if (className) node.className = className;
  return node;
}

function status(message, kind = '') {
  el('status').hidden = !message;
  el('status').classList.toggle('error', kind === 'error');
  el('status-text').textContent = message;
  el('spinner').hidden = kind !== 'running';
}

function captureSlot(when) { return `${when}_${el('capture-kind').value}`; }

function controls() {
  for (const button of document.querySelectorAll('button[data-action],.nav-button')) button.disabled = busy;
  el('capture-kind').disabled = busy;
  el('start-scan').disabled ||= !selections.has('image');
  el('start-repair').disabled ||= !selections.has('damaged');
  el('start-transfer').disabled ||= !selections.has('transfer');
  el('start-compare').disabled ||= !selections.has(captureSlot('before')) || !selections.has(captureSlot('after'));
  el('save-finding').disabled ||= activeFinding === null;
  el('save-repaired').disabled ||= repairReport?.status !== 'repair_available';
  el('read-hex').disabled ||= !scanReport;
  window.syncWorkspaceControls?.();
}

async function run(progress, action) {
  if (busy) return;
  busy = true;
  controls();
  status(progress, 'running');
  try {
    const message = await action();
    status(message || '');
  } catch (failure) {
    const message = typeof failure === 'string' ? failure : failure?.message || 'The operation could not be completed.';
    status(message, 'error');
  } finally {
    busy = false;
    controls();
  }
}

function setSelection(slot, selected, label, placeholder) {
  if (selected) selections.set(slot, selected);
  const current = selections.get(slot);
  el(label).textContent = current ? `${current.name}${current.bytes === null ? '' : ` · ${size(current.bytes)}`}` : placeholder;
}

function pick(button, slot, label, reset) {
  el(button).addEventListener('click', () => run('Opening file picker…', async () => {
    const key = typeof slot === 'function' ? slot() : slot;
    const chosen = await invoke('pick_source', { slot: key });
    if (!chosen) return;
    setSelection(key, chosen, label, 'Choose a source.');
    reset();
  }));
}

function showView(id) {
    for (const view of document.querySelectorAll('.view')) view.hidden = view.id !== id;
    for (const nav of document.querySelectorAll('[data-view]')) {
      nav.classList.toggle('active', nav.dataset.view === id);
      if (nav.dataset.view === id) nav.setAttribute('aria-current', 'page');
      else nav.removeAttribute('aria-current');
    }
    if (id !== 'transfer') {
      el('password').value = '';
      el('confirm-password').value = '';
    }
    status('');
    document.dispatchEvent(new CustomEvent('view-changed', {detail: id}));
}

for (const button of document.querySelectorAll('[data-view]')) {
  button.addEventListener('click', () => showView(button.dataset.view));
}

function describe(target, fields) {
  el(target).replaceChildren(...fields.map(([name, value, wide]) => {
    const group = text('div', '', wide ? 'detail wide' : 'detail');
    group.append(text('dt', name), text('dd', value));
    return group;
  }));
}

function metrics(target, fields) {
  el(target).replaceChildren(...fields.map(([name, value]) => {
    const card = text('div', '', 'metric');
    card.append(text('span', name), text('strong', value));
    return card;
  }));
}

function clearPreview() {
  activeFinding = null;
  el('finding-image').hidden = true;
  el('finding-image').removeAttribute('src');
  el('preview-message').hidden = false;
  el('finding-hex-detail').hidden = true;
  el('finding-hex-detail').open = false;
  el('finding-detail').replaceChildren();
}

function resetScan() {
  scanReport = null;
  clearPreview();
  el('scan-output').hidden = true;
  el('source-hex-panel').open = false;
  el('source-hex').hidden = true;
  el('source-hex').textContent = '';
}

function readBound(input, fallback) {
  const value = el(input).value.trim();
  if (!value) return fallback;
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(Number(value))) throw Error('Enter a whole byte offset and length.');
  return Number(value);
}

async function scan() {
  resetScan();
  scanReport = await invoke('scan_image', {storage: el('storage').value, offset: readBound('offset', 0), length: readBound('length', null), media: el('scan-media').value || null});
  metrics('scan-metrics', [
    ['Validated findings', number.format(scanReport.findings.length)],
    ['Rejected candidates', number.format(scanReport.rejected.length)],
    ['Observed bytes scanned', size(scanReport.observed_bytes_scanned)],
  ]);
  el('scan-output').hidden = false;
  renderFindings();
  const gaps = scanReport.fill_bytes_skipped + scanReport.unspecified_bytes_skipped;
  const coverage = scanReport.observed_ranges_fully_scanned ? 'Scan finished.' : 'Scan reached its validation limit. Coverage is partial.';
  const scope = gaps ? `${coverage} ${size(gaps)} of sparse ranges were skipped.` : coverage;
  return scanReport.logging_warning ? `${scope} ${scanReport.logging_warning}` : scope;
}

function originOf(finding) {
  const partitions = scanReport.partition_map?.partitions || [];
  return partitions.find(partition => finding.offset >= partition.offset && finding.offset + finding.length <= partition.offset + partition.length);
}

function renderFindings() {
  if (!scanReport) return;
  const type = el('type-filter').value;
  const order = el('sort-findings').value;
  const origin = el('origin-filter').value;
  const rows = scanReport.findings.map((finding, index) => ({finding, index})).filter(({finding}) => (!type || finding.media_type === type) && (!origin || (originOf(finding)?.provenance || 'image_unassigned') === origin));
  rows.sort((a, b) => order === 'offset' ? a.finding.offset - b.finding.offset : b.finding[order] - a.finding[order]);
  el('findings').replaceChildren(...rows.map(({finding, index}) => {
    const row = document.createElement('tr');
    const cell = document.createElement('td');
    const button = text('button', finding.media_type.toUpperCase());
    button.dataset.action = '';
    button.disabled = busy;
    button.setAttribute('aria-label', `Preview ${finding.media_type.toUpperCase()} at byte ${finding.offset}`);
    button.addEventListener('click', () => run('Re-verifying finding…', () => preview(index)));
    cell.append(button);
    row.append(cell, text('td', number.format(finding.offset)), text('td', size(finding.length)), text('td', `${finding.confidence} / 4`));
    return row;
  }));
  el('no-findings').hidden = rows.length !== 0;
}

async function preview(index) {
  clearPreview();
  const finding = scanReport.findings[index];
  const media = await invoke('preview_media', {repair: false, index});
  activeFinding = index;
  el('finding-image').src = media.url;
  el('finding-image').hidden = false;
  el('preview-message').hidden = true;
  el('finding-hex').textContent = media.hex;
  el('finding-hex-detail').hidden = false;
  describe('finding-detail', [
    ['Dimensions', `${finding.width} × ${finding.height}`], ['Source offset', number.format(finding.offset)],
    ['Source origin', originOf(finding) ? `${words(originOf(finding).provenance)} · ${originOf(finding).name}` : 'Unassigned image bytes', true],
    ['Validation', words(finding.reason), true], ['SHA-256', finding.sha256, true],
  ]);
  return 'Preview verified against the source.';
}

pick('pick-image', 'image', 'image-name', resetScan);
el('demo-image').addEventListener('click', () => run('Scanning the demo image…', async () => {
  setSelection('image', await invoke('demo_image'), 'image-name', 'Choose an image.');
  el('storage').value = 'memory-card';
  el('scan-media').value = '';
  el('offset').value = '';
  el('length').value = '';
  el('type-filter').value = '';
  el('origin-filter').value = '';
  await scan();
  if (scanReport.findings.length) await preview(0);
  return `Demo complete. Validated findings: ${scanReport.findings.length}.`;
}));
el('start-scan').addEventListener('click', () => run('Scanning and verifying the image…', scan));
el('type-filter').addEventListener('change', renderFindings);
el('origin-filter').addEventListener('change', renderFindings);
el('sort-findings').addEventListener('change', renderFindings);
el('read-hex').addEventListener('click', () => run('Re-verifying the selected source bytes…', async () => {
  el('source-hex').hidden = true;
  el('source-hex').textContent = await invoke('inspect_bytes', {offset: readBound('hex-offset', 0), length: readBound('hex-length', 64)});
  el('source-hex').hidden = false;
  return 'Source bytes verified.';
}));

for (const [id, kind] of [['save-scan-report','scan'], ['save-compare-report','compare'], ['save-repair-report','repair']]) {
  el(id).addEventListener('click', () => run('Choose where to save the report…', async () => {
    const name = await invoke('save_report', {kind});
    return name ? `Saved ${name}.` : '';
  }));
}

for (const [id, repair] of [['save-finding', false], ['save-repaired', true]]) {
  el(id).addEventListener('click', () => run('Choose a destination on a different physical drive…', async () => {
    const name = await invoke('save_media', {repair, index: repair ? 0 : activeFinding});
    return name ? `Verified and saved ${name}.` : '';
  }));
}

function resetCompare() { el('compare-output').hidden = true; }
pick('pick-before', () => captureSlot('before'), 'before-name', resetCompare);
pick('pick-after', () => captureSlot('after'), 'after-name', resetCompare);
el('capture-kind').addEventListener('change', () => {
  setSelection(captureSlot('before'), null, 'before-name', 'Choose the earlier capture.');
  setSelection(captureSlot('after'), null, 'after-name', 'Choose the later capture.');
  resetCompare();
  controls();
});
el('start-compare').addEventListener('click', () => run('Verifying both captures…', async () => {
  resetCompare();
  const result = await invoke('compare_sources', {folders: el('capture-kind').value === 'folder'});
  return renderComparison(result);
}));
function renderComparison(result, source = 'Independently acquired sources') {
  const changes = result.paths ? Object.entries(result.paths).map(([name, change]) => [name, change.observation]) : result.changes.map(change => [`${change.media_type.toUpperCase()} · ${change.sha256.slice(0, 16)}…`, change.observation]);
  el('compare-rows').replaceChildren(...changes.slice(0, 250).map(([name, observation]) => {
    const row = document.createElement('tr');
    row.append(text('td', name), text('td', words(observation)));
    return row;
  }));
  el('compare-summary').textContent = `${source} · ${number.format(changes.length)} observations. ${changes.length > 250 ? 'Showing the first 250. The saved report includes all observations.' : ''}`;
  el('compare-output').hidden = false;
  return 'Comparison finished. Capture order is supplied by the operator.';
}

function resetRepair() {
  repairReport = null;
  el('repair-output').hidden = true;
  el('repair-picture').hidden = true;
  el('repair-image').removeAttribute('src');
  el('save-repaired').hidden = true;
}
pick('pick-damaged', 'damaged', 'damaged-name', resetRepair);
el('start-repair').addEventListener('click', () => run('Checking file structure and repair candidates…', async () => {
  resetRepair();
  repairReport = await invoke('repair_file', {expected: el('original-hash').value.trim() || null});
  describe('repair-detail', [
    ['Status', words(repairReport.status)], ['Reason', words(repairReport.reason)],
    ['Verification', words(repairReport.result?.verification)], ['Method', words(repairReport.result?.change?.algorithm)],
    ['Source SHA-256', repairReport.source_sha256, true],
  ]);
  el('repair-output').hidden = false;
  if (repairReport.status === 'repair_available') {
    const media = await invoke('preview_media', {repair: true, index: 0});
    el('repair-image').src = media.url;
    el('repair-picture').hidden = false;
    el('save-repaired').hidden = !mediaSaving;
  }
  return 'Repair assessment finished.';
}));

pick('pick-transfer', 'transfer', 'transfer-name', () => { el('transfer-result').hidden = true; window.clearPhoneSelection?.(); });
el('start-transfer').addEventListener('click', () => run('Choose where to save the archive…', async () => {
  const password = el('password').value;
  if (!/^[ -~]{12,128}$/.test(password)) throw Error('Enter a passphrase of 12–128 printable ASCII characters.');
  if (password !== el('confirm-password').value) throw Error('The passphrases do not match.');
  el('password').value = '';
  el('confirm-password').value = '';
  const result = await invoke('create_transfer', {password, selection: window.phoneSelection?.() ?? null});
  if (!result) return;
  el('transfer-result').textContent = `${number.format(result.files)} files packaged. Archive size ${size(result.archive_bytes)}.`;
  el('transfer-result').hidden = false;
  return 'Encrypted archive created.';
}));

controls();
invoke('media_saving_available').then(available => {
  mediaSaving = available;
  el('save-finding').hidden = !available;
  el('save-repaired').hidden = !available || repairReport?.status !== 'repair_available';
}).catch(() => {});
