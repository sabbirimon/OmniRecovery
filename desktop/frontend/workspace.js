'use strict';

const catalogueNames = {
  photo: 'Photos', video: 'Videos', audio: 'Audio', document: 'Documents', log: 'Logs',
  contact: 'Contacts', message: 'SMS & messages', call_history: 'Call history', chat_history: 'Chat exports',
  whatsapp: 'WhatsApp', messenger: 'Messenger', mail: 'Mail', device_info: 'Device info & IMEI files', database: 'Databases',
  account: 'Account exports', browser: 'Browser history & tabs', archive: 'Archives & app packages', other: 'Other files'
};
const sourceNames = {
  folder: 'Folder', file: 'Single file', volume: 'Active mounted volume',
  android_export: 'Android export folder', ios_export: 'iOS export folder'
};
const sourceSlot = () => el('scan-source').value;
const captureSource = () => `capture_${el('capture-source-kind').value}`;
const inTrash = () => el('log-view').value === 'trash';
let catalogueResult = null;
let catalogueItem = null;
let cloneList = null;
let cloneSelection = new Set();
let deviceList = null;
let snapshots = null;
let activeLog = null;
let evidenceReportKind = null;
let canCopy = false;

window.syncWorkspaceControls = () => {
  el('start-catalogue').disabled ||= !selections.has(sourceSlot());
  el('recover-catalogue-item').disabled ||= !catalogueItem;
  el('capture-before').disabled ||= !selections.has(captureSource());
  el('capture-after').disabled ||= !selections.has(captureSource());
  el('clone-image').disabled ||= !selections.has('capture_image');
  el('start-acquisition').disabled ||= !el('acquisition-device').value;
  el('compare-saved-captures').disabled ||= !snapshots?.before || !snapshots?.after;
  el('review-transfer-files').disabled ||= !selections.has('transfer');
  el('start-transfer').disabled ||= cloneList !== null && cloneSelection.size === 0;
  el('verify-evidence').disabled ||= !selections.has('evidence');
  el('scan-evidence').disabled ||= !selections.has('evidence');
  el('clone-evidence').disabled ||= !selections.has('evidence');
  el('scan-sample').disabled ||= !selections.has('security');
  el('scan-security-log').disabled ||= !selections.has('security');
  el('make-sanitize-plan').disabled ||= !el('sanitize-disk').value;
  el('export-log').disabled ||= !activeLog;
  el('move-log').disabled ||= !activeLog;
  for (const id of ['scan-source', 'capture-source-kind', 'log-view', 'sanitize-disk', 'acquisition-transport', 'acquisition-device', 'acquisition-scope']) el(id).disabled = busy;
};

function resetCatalogue() {
  catalogueResult = null;
  catalogueItem = null;
  el('catalogue-output').hidden = true;
  el('catalogue-preview').hidden = true;
  el('catalogue-image').removeAttribute('src');
}

function scanSourceChanged() {
  const slot = sourceSlot();
  el('image-scan').hidden = slot !== 'image';
  el('catalogue-scan').hidden = slot === 'image';
  if (slot !== 'image') {
    setSelection(slot, null, 'catalogue-source-name', slot === 'volume' ? 'Choose a mounted volume from Devices & storage.' : `Choose ${slot === 'file' ? 'a' : 'an existing'} ${sourceNames[slot].toLowerCase()}.`);
    el('pick-catalogue-source').textContent = slot === 'volume' ? 'Open Devices & storage' : slot === 'file' ? 'Choose file' : 'Choose folder';
    el('catalogue-source-note').textContent = slot.endsWith('_export')
      ? 'Import files from a readable export already on this computer. Categories use exported filenames.'
      : 'Lists currently accessible files. Classifications use names and extensions.';
  }
  resetCatalogue();
  controls();
}

for (const tile of document.querySelectorAll('[data-jump]')) {
  tile.addEventListener('click', () => {
    if (busy) return;
    if (tile.dataset.scanSource) {
      el('scan-source').value = tile.dataset.scanSource;
      scanSourceChanged();
    }
    showView(tile.dataset.jump);
  });
}
el('scan-source').addEventListener('change', scanSourceChanged);

for (const [key, name] of Object.entries(catalogueNames)) {
  const label = text('label', '', 'category-option');
  const checkbox = document.createElement('input');
  checkbox.type = 'checkbox';
  checkbox.value = key;
  checkbox.checked = true;
  label.append(checkbox, text('span', name));
  el('catalogue-categories').append(label);
}

el('pick-catalogue-source').addEventListener('click', () => {
  const slot = sourceSlot();
  if (slot === 'volume') return showView('devices');
  run('Opening source picker…', async () => {
    const chosen = await invoke('pick_source', {slot});
    if (chosen) {
      setSelection(slot, chosen, 'catalogue-source-name', 'Choose a source.');
      resetCatalogue();
    }
  });
});

el('start-catalogue').addEventListener('click', () => run('Cataloguing selected file types…', async () => {
  const categories = [...el('catalogue-categories').querySelectorAll('input:checked')].map(box => box.value);
  if (!categories.length) throw Error('Select at least one file type.');
  resetCatalogue();
  const slot = sourceSlot();
  const result = await invoke('scan_catalogue', {slot, categories, query: el('catalogue-query').value.trim()});
  catalogueResult = {slot, result};
  el('catalogue-summary').textContent = `${number.format(result.entries.length)} matches · ${words(result.source_kind)} · ${number.format(result.skipped)} skipped${result.limit_reached ? ' · scan limit reached' : ''}${result.entries.length > 400 ? ' · showing first 400' : ''}`;
  el('catalogue-rows').replaceChildren(...result.entries.slice(0, 400).map((entry, index) => {
    const row = document.createElement('tr');
    const name = document.createElement('td');
    const button = text('button', entry.path);
    button.dataset.action = '';
    button.title = entry.path;
    button.addEventListener('click', () => inspectEntry(index));
    name.append(button);
    row.append(name, text('td', catalogueNames[entry.category]), text('td', size(entry.length)));
    return row;
  }));
  el('catalogue-output').hidden = false;
  return result.logging_warning || 'File catalogue finished.';
}));

function inspectEntry(index) {
  const job = catalogueResult;
  if (!job) return;
  run('Hashing the selected file…', async () => {
    catalogueItem = null;
    el('catalogue-preview').hidden = true;
    const item = await invoke('inspect_catalogue_entry', {slot: job.slot, index});
    catalogueItem = {slot: job.slot, index};
    el('catalogue-item-title').textContent = job.result.entries[index].path;
    describe('catalogue-item-detail', [['Source origin', words(job.result.source_kind)], ['Bytes', size(item.bytes)], ['SHA-256', item.sha256, true]]);
    el('catalogue-picture').hidden = !item.preview;
    if (item.preview) el('catalogue-image').src = item.preview;
    else el('catalogue-image').removeAttribute('src');
    el('recover-catalogue-item').hidden = !canCopy;
    el('catalogue-preview').hidden = false;
    return 'Selected file verified.';
  });
}

el('recover-catalogue-item').addEventListener('click', () => run('Choose a destination on a different physical drive…', async () => {
  const result = await invoke('copy_catalogue_entry', catalogueItem);
  return result ? `Saved ${result.name}. Verified SHA-256 ${result.sha256}.` : '';
}));

async function updateDevices() {
  deviceList = await invoke('discover_devices');
  const disks = deviceList.disks;
  const count = disks.flatMap(disk => disk.volumes).filter(volume => volume.mount).length;
  const storageNote = el('storage-inventory-note');
  storageNote.textContent = deviceList.storage_error || '';
  storageNote.hidden = !deviceList.storage_error;
  metrics('device-metrics', [['Physical drives', disks.length ? number.format(disks.filter(disk => disk.physical).length) : 'Unavailable'], ['Mounted volumes', disks.length ? number.format(count) : 'Unavailable'], ['USB devices', deviceList.usb_error ? 'Unavailable' : number.format(deviceList.usb.length)]]);
  el('disk-list').replaceChildren(...disks.map(diskCard));
  el('usb-list').replaceChildren(...deviceList.usb.map(device => {
    const row = text('div', '', 'device-row');
    row.append(text('strong', device.name), text('span', `${device.platform} · ${device.vendor}`, 'muted'));
    return row;
  }));
  if (deviceList.usb_error) el('usb-list').append(text('p', deviceList.usb_error, 'muted'));
  const list = el('sanitize-disk');
  const selected = list.value;
  list.replaceChildren(new Option('Choose a drive', ''), ...disks.filter(disk => disk.physical && !disk.internal).map(disk => new Option(`${disk.name} · ${disk.id} · ${size(disk.size)}`, disk.id)));
  list.value = [...list.options].some(option => option.value === selected) ? selected : '';
}

function diskCard(disk) {
  const details = text('details', '', 'disk-card');
  details.open = disk.volumes.some(volume => volume.selectable);
  const summary = text('summary', '', 'disk-heading');
  summary.append(text('strong', disk.name), text('span', [disk.id, size(disk.size), disk.bus, disk.physical ? (disk.internal ? 'Internal drive' : 'External drive') : 'Logical or virtual'].filter(Boolean).join(' · '), 'muted'));
  details.append(summary);
  for (const volume of disk.volumes) {
    const row = text('div', '', 'volume-row');
    const name = text('div', '', 'volume-label');
    name.append(text('strong', volume.name), text('span', [volume.id, volume.format, volume.mount || 'Unmounted', volume.writable === false ? 'Read-only' : null].filter(Boolean).join(' · '), 'muted'));
    if (Number.isFinite(volume.size) && Number.isFinite(volume.free) && volume.size > 0 && volume.free <= volume.size) {
      const progress = document.createElement('progress');
      progress.max = 100;
      progress.value = (volume.size - volume.free) / volume.size * 100;
      progress.setAttribute('aria-label', `${volume.name} used space`);
      name.append(progress, text('span', `${size(volume.free)} available`, 'muted'));
    }
    row.append(name);
    if (volume.selectable) {
      const button = text('button', 'Scan files');
      button.dataset.action = '';
      button.addEventListener('click', () => run('Rechecking the selected volume…', async () => {
        setSelection('volume', await invoke('choose_volume', {id: volume.id}), 'catalogue-source-name', 'Choose a volume.');
        el('scan-source').value = 'volume';
        scanSourceChanged();
        showView('scan');
        return 'Volume selected. Choose file types and start the scan.';
      }));
      row.append(button);
    }
    details.append(row);
  }
  return details;
}

for (const id of ['refresh-devices', 'refresh-sanitize']) el(id).addEventListener('click', () => run('Reading connected devices…', updateDevices));
el('check-adb').addEventListener('click', () => run('Checking the local Android USB bridge…', async () => {
  const devices = await invoke('check_adb');
  el('adb-results').replaceChildren(...devices.map(device => {
    const row = text('div', '', 'device-row');
    row.append(text('strong', device.model), text('span', words(device.state), 'muted'));
    return row;
  }));
  return devices.length ? 'ADB status updated. Approve the USB debugging prompt on the phone if shown.' : 'No USB Android device was reported by ADB.';
}));

el('acquisition-transport').addEventListener('change', () => {
  el('acquisition-device').replaceChildren(new Option('Refresh to detect devices', ''));
  el('acquisition-scope-label').hidden = el('acquisition-transport').value !== 'android';
  el('acquisition-status').textContent = 'Detect connected devices for this transport.';
  window.syncWorkspaceControls();
});
el('acquisition-device').addEventListener('change', () => {
  el('start-acquisition').disabled = busy || !el('acquisition-device').value;
});
el('refresh-acquisition').addEventListener('click', () => run('Detecting USB devices…', async () => {
  const list = el('acquisition-device');
  list.replaceChildren(new Option('Choose a connected device', ''));
  const devices = await invoke('list_acquisition_devices', {transport: el('acquisition-transport').value});
  const options = devices.map(device => {
    const option = new Option([device.model, device.id, words(device.state)].filter(Boolean).join(' · '), device.id);
    option.disabled = !['device', 'attached'].includes(device.state);
    return option;
  });
  list.append(...options);
  el('acquisition-status').textContent = devices.length ? 'Choose an attached, authorized device.' : 'No connected device was reported by the selected USB transport.';
  return el('acquisition-status').textContent;
}));
el('start-acquisition').addEventListener('click', () => run('Capturing and verifying device files…', async () => {
  const result = await invoke('capture_usb_device', {
    transport: el('acquisition-transport').value,
    id: el('acquisition-device').value,
    scope: el('acquisition-scope').value
  });
  if (!result) return '';
  el('acquisition-status').textContent = `Saved ${number.format(result.files)} verified files · ${size(result.bytes)} · ${result.output}`;
  return 'USB capture saved with its SHA-256 manifest.';
}));

function renderCapture() {
  for (const stage of ['before', 'after']) {
    const item = snapshots?.[stage];
    el(`${stage}-capture-state`).textContent = item ? `${item.name} · ${new Date(item.captured_ms).toLocaleString()} · ${number.format(item.entries_count)} entries${item.incomplete ? ' · partial scope' : ''}` : 'No capture saved.';
  }
  controls();
}

async function refreshCaptures() { snapshots = await invoke('capture_status'); renderCapture(); }
pick('pick-capture-source', captureSource, 'capture-source-name', () => {});
el('capture-source-kind').addEventListener('change', () => {
  const image = captureSource() === 'capture_image';
  setSelection(captureSource(), null, 'capture-source-name', image ? 'Choose an existing disk image to begin.' : 'Choose a folder or phone export to begin.');
  el('clone-image').hidden = !canCopy || !image;
  controls();
});
for (const stage of ['before', 'after']) {
  el(`capture-${stage}`).addEventListener('click', () => run(`Taking ${stage} capture…`, async () => {
    if (snapshots?.[stage] && !window.confirm(`Replace the saved ${stage} capture?`)) return;
    const result = await invoke('take_capture', {stage, folder: captureSource() === 'capture_folder'});
    await refreshCaptures();
    return `${stage === 'before' ? 'Before' : 'After'} capture saved.${result.logging_warning ? ` ${result.logging_warning}` : ''}`;
  }));
}
el('compare-saved-captures').addEventListener('click', () => run('Comparing the saved pair…', async () => {
  const result = await invoke('compare_saved');
  showView('compare');
  return renderComparison(result, 'Saved before/after pair');
}));
el('clone-image').addEventListener('click', () => run('Choose a separate physical drive for the clone…', async () => {
  const result = await invoke('clone_image');
  return result ? `Saved image clone ${result.name}. Verified SHA-256 ${result.sha256}.` : '';
}));

window.clearPhoneSelection = () => {
  cloneList = null;
  cloneSelection.clear();
  el('transfer-selection').hidden = true;
};
window.phoneSelection = () => cloneList === null ? null : [...cloneSelection];

function renderCloneSelection() {
  el('transfer-selection-summary').textContent = `${number.format(cloneSelection.size)} selected out of ${number.format(cloneList.entries.length)}. ${cloneList.entries.length > 400 ? 'Showing first 400.' : ''}${cloneList.limit_reached ? ' Catalogue limit reached.' : ''}`;
  el('toggle-transfer-selection').textContent = cloneSelection.size === cloneList.entries.length ? 'Clear selection' : 'Select all';
  controls();
}

function renderCloneList() {
  el('transfer-rows').replaceChildren(...cloneList.entries.slice(0, 400).map((entry, index) => {
    const row = document.createElement('tr');
    const toggle = document.createElement('input');
    toggle.type = 'checkbox';
    toggle.checked = cloneSelection.has(index);
    toggle.setAttribute('aria-label', `Copy ${entry.path}`);
    toggle.addEventListener('change', () => {
      if (toggle.checked) cloneSelection.add(index);
      else cloneSelection.delete(index);
      renderCloneSelection();
    });
    const first = document.createElement('td');
    first.append(toggle);
    row.append(first, text('td', entry.path), text('td', catalogueNames[entry.category]));
    return row;
  }));
  el('transfer-selection').hidden = false;
  renderCloneSelection();
}

el('review-transfer-files').addEventListener('click', () => run('Reading the phone export catalogue…', async () => {
  cloneList = await invoke('scan_catalogue', {slot: 'transfer', categories: [], query: ''});
  cloneSelection = new Set(cloneList.entries.map((_, index) => index));
  renderCloneList();
  return 'Select the files to clone.';
}));
el('toggle-transfer-selection').addEventListener('click', () => {
  if (!cloneList) return;
  cloneSelection = cloneSelection.size === cloneList.entries.length ? new Set() : new Set(cloneList.entries.map((_, index) => index));
  renderCloneList();
});

function showEvidenceReport(kind, title, fields) {
  evidenceReportKind = kind;
  el('evidence-report-title').textContent = title;
  describe('evidence-detail', fields);
  el('evidence-output').hidden = false;
}

const caseReference = () => el('case-reference').value.trim();
pick('pick-evidence', 'evidence', 'evidence-name', () => { el('evidence-output').hidden = true; });
el('verify-evidence').addEventListener('click', () => run('Verifying the selected evidence file…', async () => {
  el('evidence-output').hidden = true;
  const result = await invoke('verify_evidence', {expected: el('evidence-sha').value.trim(), caseReference: caseReference()});
  showEvidenceReport('evidence', 'Verification report', [['Case reference', result.case_reference || 'Unspecified'], ['File', result.source], ['Bytes', size(result.bytes)], ['Read passes', result.verified_passes], ['Reference match', result.matches_reference === null ? 'No reference supplied' : result.matches_reference ? 'Matched' : 'Mismatch'], ['SHA-256', result.sha256, true]]);
  return result.logging_warning || 'Verification finished.';
}));

el('scan-evidence').addEventListener('click', () => run('Scanning the selected evidence image…', async () => {
  el('evidence-output').hidden = true;
  const result = await invoke('scan_evidence', {caseReference: caseReference(), offset: readBound('evidence-offset', 0), length: readBound('evidence-length', null), media: el('evidence-media').value || null});
  showEvidenceReport('forensic_scan', 'Forensic scan report', [
    ['Case reference', result.case_reference || 'Unspecified'], ['Container', words(result.container)],
    ['Byte offset', number.format(result.selected_range.offset)], ['Scope', size(result.selected_range.length)],
    ['Observed bytes scanned', size(result.observed_bytes_scanned)], ['Sparse bytes skipped', size(result.fill_bytes_skipped + result.unspecified_bytes_skipped)],
    ['Observed range coverage', result.observed_ranges_fully_scanned ? 'Complete' : 'Partial'], ['Media filter', result.media_filter?.toUpperCase() || 'All supported media'],
    ['Validated findings', number.format(result.findings.length)], ['Rejected candidates', number.format(result.rejected.length)],
    ['Decode-verified', number.format(result.triage.decode_verified)], ['Container-verified', number.format(result.triage.container_verified)],
    ['Cut short at a source boundary', number.format(result.triage.needs_more_source)], ['Exceeded validation limits', number.format(result.triage.validation_limited)],
    ['Elapsed', `${number.format(result.elapsed_ms)} ms`], ['Source SHA-256', result.source_sha256, true],
  ]);
  return result.logging_warning || 'Forensic scan finished. Export the report for individual findings.';
}));

el('clone-evidence').addEventListener('click', () => run('Choose a separate physical drive for the clone…', async () => {
  el('evidence-output').hidden = true;
  const result = await invoke('clone_evidence', {caseReference: caseReference()});
  if (result.status === 'cancelled') return result.logging_warning || '';
  showEvidenceReport('evidence_clone', 'Image clone report', [
    ['Case reference', result.case_reference || 'Unspecified'], ['Output file', result.name], ['Bytes copied', size(result.bytes)],
    ['Readback verification', result.readback_verified ? 'Passed' : 'Unavailable'], ['Elapsed', `${number.format(result.elapsed_ms)} ms`], ['SHA-256', result.sha256, true],
  ]);
  return result.logging_warning || 'Clone saved and verified.';
}));

el('save-evidence-report').addEventListener('click', () => run('Choose where to export the report…', async () => {
  const name = await invoke('save_report', {kind: evidenceReportKind});
  return name ? `Saved ${name}.` : '';
}));

pick('pick-security', 'security', 'security-name', () => { el('security-output').hidden = true; });
for (const [id, log] of [['scan-sample', false], ['scan-security-log', true]]) {
  el(id).addEventListener('click', () => run(log ? 'Checking log indicators…' : 'Running the local malware scanner…', async () => {
    el('security-output').hidden = true;
    const result = await invoke('security_scan', {log});
    describe('security-detail', [['Engine', words(result.engine)], ['Result', words(result.result)], ['SHA-256', result.sha256, true]]);
    el('security-indicators').hidden = result.indicators.length === 0;
    el('security-rows').replaceChildren(...result.indicators.map(item => {
      const row = document.createElement('tr');
      row.append(text('td', item.line), text('td', words(item.kind)));
      return row;
    }));
    el('security-output').hidden = false;
    return result.logging_warning || 'Local analysis finished.';
  }));
}

async function refreshLogs() {
  activeLog = null;
  el('log-detail').hidden = true;
  const logs = await invoke('activity_logs', {trash: inTrash()});
  el('logs-empty').hidden = logs.length > 0;
  el('log-rows').replaceChildren(...logs.map(log => {
    const row = document.createElement('tr');
    const cell = document.createElement('td');
    const button = text('button', words(log.operation));
    button.dataset.action = '';
    button.addEventListener('click', () => {
      activeLog = log;
      el('log-json').textContent = JSON.stringify(log, null, 2);
      el('move-log').textContent = inTrash() ? 'Restore log' : 'Delete log';
      el('log-detail').hidden = false;
      controls();
    });
    cell.append(button);
    row.append(cell, text('td', new Date(log.started_ms).toLocaleString()), text('td', words(log.outcome)));
    return row;
  }));
}
el('refresh-logs').addEventListener('click', () => run('Loading scan logs…', refreshLogs));
el('log-view').addEventListener('change', () => run('Loading scan logs…', refreshLogs));
el('move-log').addEventListener('click', () => run('Updating the activity log…', async () => {
  const trash = inTrash();
  if (!trash && !window.confirm('Move this activity log to Deleted logs?')) return;
  await invoke('move_activity_log', {id: activeLog.id, fromTrash: trash});
  await refreshLogs();
  return trash ? 'Activity log restored.' : 'Activity log moved to Deleted logs.';
}));
el('export-log').addEventListener('click', () => run('Choose where to export the activity log…', async () => {
  const name = await invoke('export_activity_log', {id: activeLog.id, trash: inTrash()});
  return name ? `Exported ${name}.` : '';
}));

el('sanitize-disk').addEventListener('change', () => { el('sanitize-output').hidden = true; controls(); });
el('make-sanitize-plan').addEventListener('click', () => run('Preparing a device-specific plan…', async () => {
  const result = await invoke('sanitize_plan', {id: el('sanitize-disk').value, goal: el('sanitize-goal').value});
  describe('sanitize-detail', [['Target', `${result.model} · ${result.device_id}`], ['Goal', words(result.goal)], ['Method guidance', result.method_guidance, true], ['Verification', result.verification, true], ['Approval', result.approval, true]]);
  el('sanitize-output').hidden = false;
  return 'Plan prepared for review.';
}));

for (const [id, kind] of [['save-catalogue-report', 'catalogue'], ['save-security-report', 'security'], ['save-sanitize-report', 'sanitize']]) {
  el(id).addEventListener('click', () => run('Choose where to export the report…', async () => {
    const name = await invoke('save_report', {kind});
    return name ? `Saved ${name}.` : '';
  }));
}

document.addEventListener('view-changed', ({detail}) => {
  if (busy) return;
  if ((detail === 'devices' || detail === 'sanitize') && !deviceList) run('Reading connected devices…', updateDevices);
  if (detail === 'capture') run('Loading saved captures…', refreshCaptures);
  if (detail === 'logs') run('Loading scan logs…', refreshLogs);
});

invoke('media_saving_available').then(available => {
  canCopy = available;
  el('clone-evidence').hidden = !available;
  el('clone-image').hidden = !available || captureSource() !== 'capture_image';
}).catch(() => {});
controls();
