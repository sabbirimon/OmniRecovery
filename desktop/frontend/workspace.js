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
let clonePhones = [];
let clonePhonesLoaded = false;
let pendingCloneSource = null;
let pickedTransferCount = 0;
let deviceList = null;
let snapshots = null;
let activeLog = null;
let evidenceReportKind = null;
let canCopy = false;
let recoveryAction = null;
let mirrorLive = false;
let storageFilter = 'all';
let storageSelection = null;
let storagePhones = [];
let storagePhoneDetails = null;
let servicePhones = [];

const preferenceKey = 'omnirecovery-workspace-settings-v1';
const defaultPreferences = {separate:true, compact:false, refresh:true, interval:5, scope:'shared', usbQuality:'balanced', wifiQuality:'low_bandwidth'};
const qualityValues = ['low_bandwidth','balanced','high_quality'];
function readPreferences() {
  try {
    const value = JSON.parse(localStorage.getItem(preferenceKey)) || {};
    return {
      separate: typeof value.separate === 'boolean' ? value.separate : true,
      compact: value.compact === true,
      refresh: typeof value.refresh === 'boolean' ? value.refresh : true,
      interval: [5,10,20].includes(value.interval) ? value.interval : 5,
      scope: ['shared','camera','documents','device_info'].includes(value.scope) ? value.scope : 'shared',
      usbQuality: qualityValues.includes(value.usbQuality) ? value.usbQuality : 'balanced',
      wifiQuality: qualityValues.includes(value.wifiQuality) ? value.wifiQuality : 'low_bandwidth',
    };
  } catch { return {...defaultPreferences}; }
}
let preferences = readPreferences();
function applyPreferences() {
  document.querySelector('.workspace').classList.toggle('sidebar-compact', preferences.compact);
  const toggle = el('sidebar-toggle');
  toggle.setAttribute('aria-pressed', String(preferences.compact));
  toggle.title = preferences.compact ? 'Expand navigation' : 'Collapse navigation';
  toggle.textContent = preferences.compact ? '›' : '‹';
  el('settings-windows').checked = preferences.separate;
  el('settings-sidebar').checked = preferences.compact;
  el('settings-refresh').checked = preferences.refresh;
  el('settings-interval').value = String(preferences.interval);
  el('settings-scope').value = preferences.scope;
  el('settings-usb-quality').value = preferences.usbQuality;
  el('settings-wifi-quality').value = preferences.wifiQuality;
}
function updatePreferences(next) {
  const previous = preferences;
  preferences = next;
  if (next.scope !== previous.scope && !busy) el('acquisition-scope').value = next.scope;
  const quality = el('mirror-transport').value === 'android_wireless' ? 'wifiQuality' : 'usbQuality';
  if (next[quality] !== previous[quality] && !busy && !mirrorLive) el('mirror-profile').value = next[quality];
  applyPreferences();
}
function savePreferences(update) {
  updatePreferences({...preferences, ...update});
  try { localStorage.setItem(preferenceKey, JSON.stringify(preferences)); } catch {}
}
el('sidebar-toggle').addEventListener('click', () => savePreferences({compact:!preferences.compact}));
for (const [id, key] of [['settings-windows','separate'],['settings-sidebar','compact'],['settings-refresh','refresh']]) {
  el(id).addEventListener('change', () => savePreferences({[key]:el(id).checked}));
}
for (const [id, key] of [['settings-interval','interval'],['settings-scope','scope'],['settings-usb-quality','usbQuality'],['settings-wifi-quality','wifiQuality']]) {
  el(id).addEventListener('change', () => savePreferences({[key]:key === 'interval' ? Number(el(id).value) : el(id).value}));
}
el('reset-settings').addEventListener('click', () => savePreferences(defaultPreferences));
window.addEventListener('storage', event => {
  if (event.key === preferenceKey) updatePreferences(readPreferences());
});
for (const button of document.querySelectorAll('.nav-button')) {
  const label = button.querySelector('.nav-label')?.textContent;
  if (label) { button.title = label; button.setAttribute('aria-label', label); }
}
applyPreferences();
el('acquisition-scope').value = preferences.scope;
el('mirror-profile').value = preferences.usbQuality;

window.syncWorkspaceControls = () => {
  el('start-catalogue').disabled ||= !selections.has(sourceSlot());
  el('read-records').disabled ||= !selections.has('artifact');
  el('save-firmware-plan').disabled ||= !selections.has('firmware');
  el('service-read-specs').disabled ||= !el('service-android-device').value;
  el('service-capture').disabled ||= !el('service-android-device').value;
  el('service-android-device').disabled = busy;
  el('service-android-transport').disabled = busy;
  el('record-kind').disabled = busy;
  el('recover-catalogue-item').disabled ||= !catalogueItem;
  el('capture-before').disabled ||= !selections.has(captureSource());
  el('capture-after').disabled ||= !selections.has(captureSource());
  el('clone-image').disabled ||= !selections.has('capture_image');
  el('start-acquisition').disabled ||= !el('acquisition-device').value;
  el('start-mirror').disabled ||= mirrorLive || !el('mirror-device').value;
  el('stop-mirror').disabled ||= !mirrorLive;
  for (const id of ['mirror-transport', 'mirror-device', 'mirror-video', 'mirror-audio', 'mirror-profile']) el(id).disabled = busy || mirrorLive;
  el('change-ios-recovery').disabled ||= !el('service-ios-device').value || !recoveryAction;
  el('service-capture-ios').disabled ||= !el('service-ios-device').value || recoveryAction !== 'enter';
  el('compare-saved-captures').disabled ||= !snapshots?.before || !snapshots?.after;
  el('review-transfer-files').disabled ||= !selections.has('transfer');
  const sourcePhone = clonePhone('transfer-source');
  const targetPhone = clonePhone('transfer-target');
  const exportSource = el('transfer-source').value === 'export';
  const sourceReady = exportSource
    ? selections.has('transfer') && (cloneList === null || cloneSelection.size > 0)
    : el('transfer-source').value === 'files' ? pickedTransferCount > 0 : sourcePhone && chosenCloneFolders().length > 0;
  const targetReady = el('transfer-target').value === 'computer' || targetPhone;
  const query = el('phone-live-query').value.trim().toLowerCase();
  const matches = currentBrowse?.report.entries.filter(item => item.path.toLowerCase().includes(query)).length || 0;
  el('phone-live-prev').disabled ||= browsePage === 0;
  el('phone-live-next').disabled ||= matches <= (browsePage + 1) * phonePageSize;
  el('phone-live-query').disabled = busy;
  el('start-transfer').disabled ||= !sourceReady || !targetReady || Boolean(sourcePhone && sourcePhone === targetPhone);
  el('flip-transfer').disabled ||= !sourcePhone || !targetPhone || sourcePhone === targetPhone;
  for (const id of ['transfer-source', 'transfer-target', 'transfer-folders']) el(id).disabled = busy;
  el('verify-evidence').disabled ||= !selections.has('evidence');
  el('scan-evidence').disabled ||= !selections.has('evidence');
  el('clone-evidence').disabled ||= !selections.has('evidence');
  el('scan-sample').disabled ||= !selections.has('security');
  el('scan-security-log').disabled ||= !selections.has('security');
  el('make-sanitize-plan').disabled ||= !el('sanitize-disk').value;
  el('save-phone-wipe-plan').disabled ||= !el('sanitize-phone').value;
  el('export-log').disabled ||= !activeLog;
  el('move-log').disabled ||= !activeLog;
  for (const id of ['scan-source', 'capture-source-kind', 'log-view', 'sanitize-disk', 'sanitize-phone', 'acquisition-transport', 'acquisition-device', 'acquisition-scope', 'service-ios-device']) el(id).disabled = busy;
  if (phoneBinding) {
    el('acquisition-transport').disabled = true;
    el('acquisition-device').disabled = true;
  }
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
  const separate = Boolean(tile.closest('#toolbox') && tile.classList.contains('tool-tile'));
  if (separate) tile.title = 'Open tool';
  tile.addEventListener('click', () => {
    if (busy) return;
    if (separate && preferences.separate) {
      run('Opening tool window…', () => invoke('open_tool_window', {
        view: tile.dataset.jump,
        scanSource: tile.dataset.scanSource || null
      }));
      return;
    }
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
  const [inventory, phones] = await Promise.allSettled([invoke('discover_devices'), invoke('discover_phones')]);
  if (inventory.status === 'rejected') throw inventory.reason;
  deviceList = inventory.value;
  storagePhones = phones.status === 'fulfilled' ? phones.value.phones : [];
  storagePhoneDetails = null;
  const disks = deviceList.disks;
  const storageNote = el('storage-inventory-note');
  storageNote.textContent = deviceList.storage_error || '';
  storageNote.hidden = !deviceList.storage_error;
  const counts = [
    ['▤', 'Physical drives', disks.filter(disk => disk.physical).length],
    ['▥', 'Mounted volumes', disks.flatMap(disk => disk.volumes).filter(volume => volume.mount).length],
    ['▯', 'Connected phones', storagePhones.length],
    ['⌁', 'USB devices', deviceList.usb_error ? null : deviceList.usb.length],
  ];
  el('device-metrics').replaceChildren(...counts.map(([symbol, label, count]) => {
    const metric = text('div', '', 'storage-stat');
    metric.append(storageIcon(symbol), text('span', label), text('strong', count == null ? 'Unavailable' : number.format(count)));
    return metric;
  }));
  el('usb-list').replaceChildren(...deviceList.usb.map(device => {
    const row = text('div', '', 'device-row');
    row.append(text('strong', device.name), text('span', `${device.platform} · ${device.vendor}`, 'muted'));
    return row;
  }));
  if (deviceList.usb_error) el('usb-list').append(text('p', deviceList.usb_error, 'muted'));
  if (storageSelection?.kind === 'phone' && !storagePhones.some(phone => phoneKey(phone) === storageSelection.id)) storageSelection = null;
  const list = el('sanitize-disk');
  const selected = list.value;
  list.replaceChildren(new Option('Choose a drive', ''), ...disks.filter(disk => disk.physical && !disk.internal).map(disk => new Option(`${disk.name} · ${disk.id} · ${size(disk.size)}`, disk.id)));
  list.value = [...list.options].some(option => option.value === selected) ? selected : '';
  const external = disks.filter(disk => disk.physical && !disk.internal);
  el('sanitize-disk-state').textContent = external.length ? `${number.format(external.length)} external physical ${external.length === 1 ? 'drive' : 'drives'} detected.` : 'No external physical drive detected. Connect a USB drive or memory card to continue.';
  const excluded = disks.filter(disk => !disk.physical || disk.internal);
  el('sanitize-exclusions').replaceChildren(...excluded.map(disk => text('p', `${disk.name} · ${disk.physical ? 'Internal drive' : 'Image or virtual volume'}`, 'muted')));
  if (!excluded.length) el('sanitize-exclusions').append(text('p', 'No other storage reported.', 'muted'));
  const eligiblePhones = storagePhones.filter(phone => phone.transport === 'android' && readyPhone(phone));
  const phoneList = el('sanitize-phone');
  const previous = phoneList.value;
  phoneList.replaceChildren(new Option('Choose an authorized phone', ''), ...eligiblePhones.map(phone => new Option(phone.name, phone.target)));
  if (eligiblePhones.some(phone => phone.target === previous)) phoneList.value = previous;
  el('sanitize-phone-state').textContent = eligiblePhones.length ? `${number.format(eligiblePhones.length)} authorized USB ${eligiblePhones.length === 1 ? 'phone' : 'phones'} detected.` : 'Connect an unlocked Android phone and approve USB debugging to create a device-bound plan.';
  renderStorage();
}

function storageIcon(symbol) {
  const icon = text('span', symbol, 'storage-icon');
  icon.setAttribute('aria-hidden', 'true');
  return icon;
}
function diskGroup(disk) { return disk.physical ? (disk.internal ? 'internal' : 'external') : 'virtual'; }
function diskSymbol(disk) { return disk.physical ? (disk.solid_state ? '▦' : '▤') : '◈'; }
function diskType(disk) {
  if (!disk.physical) return disk.bus === 'Disk Image' ? 'Disk image' : 'Virtual storage';
  return `${disk.internal ? 'Internal' : 'External'} ${disk.solid_state === true ? 'SSD' : disk.solid_state === false ? 'HDD' : 'drive'}`;
}
function selectionButton(title, subtitle, symbol, selected, action) {
  const button = text('button', '', 'storage-row');
  button.dataset.action = '';
  button.setAttribute('aria-pressed', String(selected));
  const labels = text('span', '', 'storage-row-labels');
  labels.append(text('strong', title), text('small', subtitle));
  button.append(storageIcon(symbol), labels, text('span', '›', 'storage-chevron'));
  button.addEventListener('click', action);
  return button;
}
function selectStorage(kind, id, volume = null) {
  storageSelection = {kind, id, volume};
  storagePhoneDetails = null;
  renderStorage();
  controls();
}

function renderStorage() {
  const visible = (deviceList?.disks || []).filter(disk => storageFilter === 'all' || diskGroup(disk) === storageFilter);
  const selectedDisk = visible.find(disk => storageSelection?.kind === 'disk' && storageSelection.id === disk.id);
  if (!selectedDisk && storageSelection?.kind !== 'phone') {
    const first = visible.find(disk => disk.volumes.some(volume => volume.capacity_scope === 'APFS shared container')) || visible[0];
    storageSelection = first ? {kind:'disk', id:first.id, volume:null} : null;
  }
  el('disk-list').replaceChildren(...visible.map(disk => {
    const group = text('div', '', 'storage-drive-group');
    const current = storageSelection?.kind === 'disk' && storageSelection.id === disk.id;
    group.append(selectionButton(disk.name, [diskType(disk), disk.bus, size(disk.size)].filter(Boolean).join(' · '), diskSymbol(disk), current && !storageSelection.volume, () => selectStorage('disk', disk.id)));
    if (current && disk.volumes.length) {
      const volumes = text('div', '', 'storage-volumes');
      for (const volume of disk.volumes) {
        volumes.append(selectionButton(volume.name, [volume.format?.toUpperCase(), volume.mount ? 'Mounted' : 'Unmounted'].filter(Boolean).join(' · '), '▥', storageSelection.volume === volume.id, () => selectStorage('disk', disk.id, volume.id)));
      }
      group.append(volumes);
    }
    return group;
  }));
  if (!visible.length) el('disk-list').append(text('p', 'No storage in this category.', 'muted'));
  el('storage-phone-list').replaceChildren(...storagePhones.map(phone => {
    const key = phoneKey(phone);
    return selectionButton(phone.name, phoneStatus(phone), '▯', storageSelection?.kind === 'phone' && storageSelection.id === key, () => selectStorage('phone', key));
  }));
  if (!storagePhones.length) el('storage-phone-list').append(text('p', 'No connected phone detected.', 'muted'));
  for (const button of el('storage-filters').querySelectorAll('button')) button.setAttribute('aria-pressed', String(button.dataset.storageFilter === storageFilter));
  renderStorageDetails();
}

function showCapacity(total, free, scope) {
  const valid = Number.isFinite(total) && Number.isFinite(free) && total > 0 && free >= 0 && free <= total;
  el('storage-chart').hidden = !valid;
  if (!valid) return;
  const used = total - free;
  el('capacity-total').textContent = size(total);
  el('capacity-scope').textContent = scope;
  el('capacity-used').textContent = `${size(used)} used or reserved`;
  el('capacity-free').textContent = `${size(free)} available`;
  const ring = el('capacity-ring');
  ring.setAttribute('aria-label', `${size(total)} capacity, ${size(used)} used or reserved, ${size(free)} available`);
  const ctx = ring.getContext('2d');
  if (!ctx) return;
  ctx.clearRect(0, 0, 200, 200);
  ctx.lineWidth = 17;
  ctx.strokeStyle = '#dceaf9';
  ctx.beginPath();
  ctx.arc(100, 100, 79, 0, 2 * Math.PI);
  ctx.stroke();
  if (used > 0) {
    ctx.strokeStyle = '#2775df';
    ctx.beginPath();
    ctx.arc(100, 100, 79, -Math.PI / 2, -Math.PI / 2 + 2 * Math.PI * used / total);
    ctx.stroke();
  }
}
function storageAction(label, action) {
  const button = text('button', label);
  button.dataset.action = '';
  button.addEventListener('click', action);
  el('storage-detail-actions').append(button);
}
function scanStorageVolume(volume) {
  run('Rechecking the selected volume…', async () => {
    setSelection('volume', await invoke('choose_volume', {id: volume.id}), 'catalogue-source-name', 'Choose a volume.');
    el('scan-source').value = 'volume';
    scanSourceChanged();
    showView('scan');
    return 'Volume selected. Choose file types and start the scan.';
  });
}
function renderStorageDetails() {
  el('storage-detail-actions').replaceChildren();
  showCapacity(null, null);
  const phone = storageSelection?.kind === 'phone' && storagePhones.find(item => phoneKey(item) === storageSelection.id);
  const disk = storageSelection?.kind === 'disk' && deviceList.disks.find(item => item.id === storageSelection.id);
  let fields = [];
  if (phone) {
    el('storage-detail-icon').textContent = '▯';
    el('storage-detail-title').textContent = phone.name;
    el('storage-detail-subtitle').textContent = phoneStatus(phone);
    fields = [['Platform', phone.transport.startsWith('android') ? 'Android' : 'iOS']];
    if (storagePhoneDetails) {
      const {properties, battery_percent: battery, display, shared_storage: storage} = storagePhoneDetails;
      fields.push(['Model', properties.model], ['System', properties.android_version && `Android ${properties.android_version}`], ['CPU architecture', properties.cpu_abi], ['Build', properties.build], ['Security patch', properties.security_patch], ['Battery', battery == null ? null : `${battery}%`], ['Display', display]);
      if (storage) showCapacity(storage.capacity, storage.available, 'Android shared storage');
    }
    if (phone.transport.startsWith('android') && readyPhone(phone)) {
      storageAction(storagePhoneDetails ? 'Refresh specs' : 'Read phone specs', () => run('Reading device specifications…', async () => {
        storagePhoneDetails = await invoke('android_details', {id: phone.target, transport: phone.transport});
        renderStorageDetails();
      }));
      storageAction('Screen mirror', () => openMirrorTool(phone.transport));
    }
  } else if (disk) {
    const volume = disk.volumes.find(item => item.id === storageSelection.volume);
    const meter = volume || disk.volumes.find(item => item.capacity_scope === 'APFS shared container') || disk.volumes.find(item => item.free != null);
    el('storage-detail-icon').textContent = volume ? '▥' : diskSymbol(disk);
    el('storage-detail-title').textContent = volume?.name || disk.name;
    el('storage-detail-subtitle').textContent = volume ? (volume.format?.toUpperCase() || 'Volume') : diskType(disk);
    if (meter) showCapacity(meter.size, meter.free, meter.capacity_scope || meter.name);
    fields = [['Identifier', volume?.id || disk.id], ['Interface', disk.bus], ['Drive size', disk.size == null ? null : size(disk.size)], ['Partition map', disk.partition_map && words(disk.partition_map)], ['Block size', disk.block_size == null ? null : size(disk.block_size)], ['Volumes', number.format(disk.volumes.length)]];
    if (volume) {
      fields.push(['Mount', volume.mount || 'Unmounted', true], ['Access', volume.writable == null ? null : volume.writable ? 'Writable' : 'Read-only']);
      if (volume.selectable) storageAction('Scan files', () => scanStorageVolume(volume));
    }
  } else {
    el('storage-detail-title').textContent = 'Selected device';
    el('storage-detail-subtitle').textContent = 'Select a drive, volume, or phone.';
  }
  describe('storage-specs', fields.filter(([, value]) => value != null));
}
for (const button of el('storage-filters').querySelectorAll('button')) {
  button.addEventListener('click', () => {
    storageFilter = button.dataset.storageFilter;
    if (deviceList) renderStorage();
    controls();
  });
}
for (const id of ['refresh-devices', 'refresh-sanitize']) el(id).addEventListener('click', () => run('Reading connected devices…', updateDevices));

let phoneMode = 'usb';
let usbPhones = [];
let wirelessPhones = [];
let selectedPhone = null;
let phoneChooserOpen = false;
let phoneBinding = null;
let boundInventoryLoaded = false;
let phonesLoaded = false;
let phoneRefreshInFlight = false;
let lastUsbRefresh = 0;

const livePhoneCategories = [
  ['▧', 'Photos', 'photos'],
  ['▷', 'Videos', 'videos'],
  ['♫', 'Music', 'music'],
  ['≡', 'Documents', 'documents'],
  ['↓', 'Downloads', 'downloads'],
  ['▤', 'Files', 'files'],
];
let currentBrowse = null;
let browsePage = 0;
let phonePanelKey = null;
let phoneSpecsRequest = null;
let dialogPhone = null;
const phonePageSize = 80;

function clearPhoneResults() {
  currentBrowse = null;
  browsePage = 0;
  el('phone-live-query').value = '';
  el('phone-live-results').hidden = true;
  el('phone-live-rows').replaceChildren();
}

function renderPhoneResults() {
  if (!currentBrowse) return;
  const query = el('phone-live-query').value.trim().toLowerCase();
  const matches = currentBrowse.report.entries
    .map((entry, index) => ({entry, index}))
    .filter(({entry}) => entry.path.toLowerCase().includes(query));
  const pages = Math.max(1, Math.ceil(matches.length / phonePageSize));
  browsePage = Math.min(browsePage, pages - 1);
  el('phone-live-count').textContent = `${number.format(matches.length)} files · Page ${browsePage + 1} of ${pages}${currentBrowse.report.partial ? ' · Partial listing' : ''}`;
  el('phone-live-rows').replaceChildren(...matches.slice(browsePage * phonePageSize, (browsePage + 1) * phonePageSize).map(({entry, index}) => {
    const row = text('tr', '');
    const actions = text('td', '');
    const save = text('button', 'Save copy');
    save.dataset.action = '';
    save.setAttribute('aria-label', `Save ${entry.path}`);
    save.addEventListener('click', () => run('Saving and verifying the selected phone file…', async () => {
      const report = await invoke('save_phone_file', {index});
      return report ? `Saved ${size(report.length)} · Verified SHA-256 ${report.sha256}` : '';
    }));
    actions.append(save);
    row.append(text('td', entry.path), text('td', catalogueNames[entry.category]), actions);
    return row;
  }));
  controls();
}

function browsePhone(phone, title, category) {
  run(`Reading ${title.toLowerCase()} on ${phone.name}…`, async () => {
    clearPhoneResults();
    const report = await invoke('browse_phone', {transport:phone.transport, id:phone.target, category});
    currentBrowse = {report};
    el('phone-live-title').textContent = `${title} on ${phone.name}`;
    el('phone-live-results').hidden = false;
    renderPhoneResults();
    el('phone-live-results').scrollIntoView({behavior:'smooth', block:'nearest'});
  });
}

el('phone-live-query').addEventListener('input', () => { browsePage = 0; renderPhoneResults(); });
el('phone-live-prev').addEventListener('click', () => { browsePage = Math.max(0, browsePage - 1); renderPhoneResults(); });
el('phone-live-next').addEventListener('click', () => { browsePage++; renderPhoneResults(); });
const exportPhoneCategories = [
  ['◈', 'Messages', 'message'],
  ['♧', 'Call history', 'call_history'],
  ['♙', 'Contacts', 'contact'],
  ['▦', 'Apps', 'archive'],
  ['↗', 'WhatsApp', 'whatsapp'],
  ['□', 'Calendar', 'document', '.ics'],
];

function phoneStatus(phone) {
  if (phone.state === 'device') return phone.transport === 'android_wireless' ? 'Wireless debugging authorized' : 'USB debugging authorized';
  if (phone.state === 'attached') return 'USB connected · device trust may be required';
  if (phone.state === 'unauthorized') return 'Approve USB debugging on the phone';
  if (phone.state === 'usb_detected') return phone.transport === 'ios' ? 'USB detected · confirm device trust' : 'USB detected · set up USB debugging';
  if (phone.state === 'offline') return 'Device detected · offline';
  return words(phone.state);
}

function readyPhone(phone) { return Boolean(phone.target && ['device', 'attached'].includes(phone.state)); }

function capturePhoneCategory(phone, scope) {
  const transport = el('acquisition-transport');
  transport.value = phone.transport;
  el('acquisition-device').replaceChildren(new Option(phone.name, phone.target));
  el('acquisition-scope-label').hidden = phone.transport === 'ios';
  el('acquisition-scope').value = scope;
  el('acquisition-status').textContent = `${phone.name} selected. Choose a destination to capture.`;
  showCaptureTab('device');
  showView('capture');
}

function phoneExportCategory(phone, category, query = '') {
  el('scan-source').value = phone.transport === 'ios' ? 'ios_export' : 'android_export';
  scanSourceChanged();
  for (const box of el('catalogue-categories').querySelectorAll('input')) box.checked = !category || box.value === category;
  el('catalogue-query').value = query;
  showView('scan');
}

function phoneCategoryButton(symbol, title, detail, action) {
  const button = text('button', '', 'phone-category');
  button.dataset.action = '';
  const icon = text('span', symbol, 'phone-category-icon');
  icon.setAttribute('aria-hidden', 'true');
  button.append(icon, text('strong', title), text('small', detail));
  button.addEventListener('click', action);
  return button;
}

function requestPhoneSpecs(phone, force = false) {
  if (!phone || !readyPhone(phone) || !phone.transport.startsWith('android')) return;
  const key = phoneKey(phone);
  if (!force && phoneSpecsRequest?.key === key) return;
  const request = {key};
  phoneSpecsRequest = request;
  el('phone-live-spec-state').textContent = 'Reading device details…';
  invoke('android_details', {id:phone.target, transport:phone.transport}).then(report => {
    if (phoneSpecsRequest !== request || selectedPhone !== key) return;
    const {properties, memory, battery_percent:battery, display, shared_storage:storage} = report;
    const fields = [['Model', properties.model || phone.name], ['Manufacturer', properties.manufacturer],
      ['Android version', properties.android_version], ['SDK', properties.sdk],
      ['Chipset', properties.chipset], ['Architecture', properties.cpu_abi], ['Display', display],
      ['Available RAM', memory && `${size(memory.available)} of ${size(memory.capacity)}`],
      ['Battery charge', battery == null ? null : `${battery}%`], ['Build', properties.build],
      ['Security patch', properties.security_patch], ['Connection', phone.transport === 'android' ? 'USB' : 'Paired Wi-Fi']];
    describe('phone-live-specs', fields.filter(([, value]) => value != null && value !== ''));
    el('phone-live-specs').hidden = false;
    el('phone-live-spec-state').textContent = '';
    el('phone-overview-storage').hidden = !storage;
    if (storage) {
      const used = storage.capacity - storage.available;
      el('phone-storage-meter').value = 100 * used / storage.capacity;
      el('phone-storage-values').textContent = `${size(used)} used or reserved · ${size(storage.available)} available · ${size(storage.capacity)} total`;
    }
  }).catch(error => {
    if (phoneSpecsRequest !== request || selectedPhone !== key) return;
    el('phone-live-spec-state').textContent = typeof error === 'string' ? error : error.message;
  });
}

const currentPhone = () => [...usbPhones, ...wirelessPhones].find(phone => phoneKey(phone) === selectedPhone);

function renderPhoneDetails(phone) {
  const key = phone ? JSON.stringify([phoneKey(phone), phone.state]) : null;
  if (phonePanelKey !== key) {
    phonePanelKey = key;
    phoneSpecsRequest = null;
    clearPhoneResults();
    el('phone-live-specs').hidden = true;
    el('phone-live-specs').replaceChildren();
    el('phone-live-spec-state').textContent = '';
    el('phone-overview-storage').hidden = true;
    el('phone-browse-options').open = false;
  }
  el('phone-detail').hidden = !phone;
  if (!phone) return;
  el('phone-name').textContent = phone.name;
  el('phone-state').textContent = phoneStatus(phone);
  const android = readyPhone(phone) && phone.transport.startsWith('android');
  const usbAndroid = android && phone.transport === 'android';
  el('phone-drop-target').hidden = !usbAndroid;
  el('phone-refresh-specs').hidden = !android;
  el('phone-browse-options').hidden = !android;
  const actions = [];
  if (android) {
    actions.push(phoneCategoryButton('▤', 'Browse files', 'Read shared storage', () => browsePhone(phone, 'Files', 'files')));
    actions.push(phoneCategoryButton('⊞', 'Backup device', 'Choose capture scope', () => capturePhoneCategory(phone, preferences.scope)));
    actions.push(phoneCategoryButton('↓', 'Export shared files', 'Save a verified capture', () => capturePhoneCategory(phone, 'shared')));
    if (usbAndroid) {
      actions.push(phoneCategoryButton('↑', 'Quick transfer', 'Choose computer files', () => selectQuickTransfer(phone)));
      actions.push(phoneCategoryButton('↗', 'Phone clone', 'Use this phone as source', () => {
        pendingCloneSource = phone.target;
        showView('transfer');
      }));
    }
    actions.push(phoneCategoryButton('▯', 'Screen mirror', 'Open dedicated window', () => openMirrorTool(phone.transport)));
    actions.push(phoneCategoryButton('◇', 'Phone companion', 'Restore an archive', () => el('phone-companion-dialog').showModal()));
  } else if (readyPhone(phone) && phone.transport === 'ios') {
    actions.push(phoneCategoryButton('↓', 'iOS backup', 'Collect device backup', () => capturePhoneCategory(phone, 'shared')));
    actions.push(phoneCategoryButton('◈', 'iPhone & iPad service', 'Check boot mode', () => showView('service-ios')));
  } else {
    actions.push(phoneCategoryButton('▯', 'Authorize phone', 'Connection instructions', () => openPhoneAuthorization(phone)));
  }
  actions.push(phoneCategoryButton('▧', 'WhatsApp exports', 'Read exported chat text', () => {
    el('record-kind').value = 'whatsapp_export';
    el('record-kind').dispatchEvent(new Event('change'));
    showView('records-review');
  }));
  actions.push(phoneCategoryButton('⌕', 'Data recovery', 'Scan an image or export', () => showView('scan')));
  actions.push(phoneCategoryButton('≡', 'Backup history', 'Review saved captures', () => {
    showCaptureTab('history');
    showView('capture');
  }));
  el('phone-quick-actions').replaceChildren(...actions);
  el('phone-categories').replaceChildren(...(android ? livePhoneCategories.map(([symbol, title, category]) =>
    phoneCategoryButton(symbol, title, 'Browse on phone', () => browsePhone(phone, title, category))) : []));
  el('phone-export-categories').replaceChildren(...exportPhoneCategories.map(([symbol, title, category, query]) =>
    phoneCategoryButton(symbol, title, 'Open from an export', () => phoneExportCategory(phone, category, query))));
  if (!el('my-devices').hidden) requestPhoneSpecs(phone);
}

el('phone-refresh-specs').addEventListener('click', () => requestPhoneSpecs(currentPhone(), true));
el('phone-drop-target').addEventListener('click', () => selectQuickTransfer(currentPhone()));

function phoneKey(phone) { return JSON.stringify([phone.transport, phone.target, phone.name]); }

function matchesPhoneBinding(phone) {
  return !phoneBinding || (phone.transport === phoneBinding.transport && phone.target === phoneBinding.id);
}

function renderPhones() {
  const phones = (phoneMode === 'usb' ? usbPhones : wirelessPhones).filter(matchesPhoneBinding);
  if (selectedPhone && !phones.some(phone => phoneKey(phone) === selectedPhone)) selectedPhone = null;
  if (!selectedPhone && phones.length === 1) selectedPhone = phoneKey(phones[0]);
  const current = phones.find(phone => phoneKey(phone) === selectedPhone);
  const showChooser = !phoneBinding && (phoneChooserOpen || !current || !readyPhone(current));
  el('phone-card-list').hidden = !showChooser;
  el('phone-mode-switch').hidden = !showChooser;
  el('choose-phone').hidden = Boolean(phoneBinding) || !current || !readyPhone(current);
  el('open-phone-window').hidden = Boolean(phoneBinding) || !current || !readyPhone(current);
  el('choose-phone').textContent = showChooser ? 'Hide device chooser' : 'Change device';
  const mode = phoneMode === 'usb' ? 'USB' : 'wireless';
  el('my-devices-title').textContent = phoneBinding ? 'Phone workspace' : 'My devices';
  el('phone-count').textContent = phoneBinding
    ? current && readyPhone(current) ? `Assigned to ${current.name}` : boundInventoryLoaded ? 'Reconnect the assigned phone and refresh.' : 'Checking the assigned phone…'
    : `${number.format(phones.length)} ${mode} ${phones.length === 1 ? 'device' : 'devices'} detected`;
  const add = text('button', '', 'phone-card phone-card-add');
  add.dataset.action = '';
  add.append(text('span', '+', 'phone-screen'), text('strong', 'Add device'), text('small', 'Connection setup'));
  add.addEventListener('click', () => openPhoneSetup());
  const select = phone => {
    phoneMode = phone.transport === 'android_wireless' ? 'wireless' : 'usb';
    selectedPhone = phoneKey(phone);
    phoneChooserOpen = false;
    if (el('my-devices').hidden) showView('my-devices');
    renderPhones();
    controls();
  };
  el('phone-card-list').replaceChildren(add, ...phones.map(phone => {
    const tile = text('button', '', 'phone-card');
    tile.dataset.action = '';
    tile.setAttribute('aria-pressed', String(current === phone));
    const art = text('span', '', `phone-screen ${phone.transport === 'ios' ? 'ios' : ''}`);
    art.setAttribute('aria-hidden', 'true');
    tile.append(art, text('strong', phone.name), text('small', phoneStatus(phone)));
    tile.addEventListener('click', () => {
      select(phone);
      if (!readyPhone(phone)) openPhoneAuthorization(phone);
    });
    return tile;
  }));
  el('phone-sidebar').replaceChildren(...[...usbPhones, ...wirelessPhones].filter(matchesPhoneBinding).map(phone => {
    const item = text('button', phone.name);
    item.dataset.action = '';
    if (phone === current && !el('my-devices').hidden) item.setAttribute('aria-current', 'true');
    item.addEventListener('click', () => {
      select(phone);
      if (!readyPhone(phone)) openPhoneAuthorization(phone);
    });
    if (phone !== current || el('my-devices').hidden || !readyPhone(phone) || !phone.transport.startsWith('android')) return [item];
    return [item, ...livePhoneCategories.map(([symbol, title, category]) => {
      const shortcut = text('button', '', 'phone-sidebar-category');
      shortcut.dataset.action = '';
      shortcut.append(text('span', symbol, 'phone-sidebar-icon'), text('span', title));
      shortcut.addEventListener('click', () => browsePhone(phone, title, category));
      return shortcut;
    })];
  }).flat());
  el('phone-usb-mode').setAttribute('aria-pressed', String(phoneMode === 'usb'));
  el('phone-wireless-mode').setAttribute('aria-pressed', String(phoneMode === 'wireless'));
  el('phone-usb-setup').hidden = phoneMode !== 'usb' || Boolean(current && readyPhone(current));
  el('phone-wireless-setup').hidden = phoneMode !== 'wireless' || (!showChooser && !(phoneBinding && !current));
  renderPhoneDetails(current);
}

el('open-phone-window').addEventListener('click', () => {
  const phone = currentPhone();
  if (readyPhone(phone || {})) {
    run('Opening a separate phone workspace…', () => invoke('open_phone_window', {transport:phone.transport, id:phone.target}));
  }
});

el('choose-phone').addEventListener('click', () => {
  phoneChooserOpen = !phoneChooserOpen;
  renderPhones();
  controls();
});

function openPhoneSetup() {
  phoneChooserOpen = true;
  renderPhones();
  const panel = el(`phone-${phoneMode}-setup`);
  panel.hidden = false;
  panel.scrollIntoView({behavior: 'smooth', block: 'nearest'});
}

function applyUsbInventory(inventory) {
  const changed = JSON.stringify(usbPhones) !== JSON.stringify(inventory.phones);
  usbPhones = inventory.phones;
  const notes = [inventory.android_error, inventory.usb_error];
  if (usbPhones.some(phone => phone.transport === 'ios')) notes.push(inventory.ios_error);
  el('phone-usb-status').textContent = notes.filter(Boolean).join(' ');
  phonesLoaded = true;
  lastUsbRefresh = Date.now();
  return changed;
}

async function loadPhones() {
  if (phoneMode === 'usb') {
    applyUsbInventory(await invoke('discover_phones'));
  } else {
    const detected = await invoke('list_acquisition_devices', {transport: 'android_wireless'});
    wirelessPhones = detected.map(phone => ({name: phone.model?.replaceAll('_', ' ') || 'Android phone', transport: 'android_wireless', state: phone.state, target: phone.id}));
  }
  if (phoneBinding) boundInventoryLoaded = true;
  renderPhones();
  return '';
}

setInterval(async () => {
  if (!preferences.refresh || busy || phoneRefreshInFlight || phoneMode !== 'usb' || el('my-devices').hidden || !phonesLoaded || Date.now() - lastUsbRefresh < preferences.interval * 1000) return;
  lastUsbRefresh = Date.now();
  phoneRefreshInFlight = true;
  try {
    const inventory = await invoke('discover_phones');
    if (!busy && phoneMode === 'usb' && !el('my-devices').hidden && applyUsbInventory(inventory)) renderPhones();
  } catch {
    // The manual refresh button can show transport errors without interrupting other work.
  } finally {
    phoneRefreshInFlight = false;
  }
}, 1000);

for (const mode of ['usb', 'wireless']) {
  el(`phone-${mode}-mode`).addEventListener('click', () => {
    phoneMode = mode;
    selectedPhone = null;
    renderPhones();
    run('Checking connected phones…', loadPhones);
  });
}
el('refresh-phones').addEventListener('click', () => run('Checking connected phones…', loadPhones));
function openPhoneAuthorization(phone) {
  dialogPhone = phone;
  const ios = phone.transport === 'ios';
  const wireless = phone.transport === 'android_wireless';
  el('phone-connect-device').textContent = phone.name;
  const steps = ios
    ? ['Connect the iPhone or iPad by USB and unlock it.', 'Tap Trust on the device if prompted.']
    : wireless
      ? ['Open Developer options and turn on Wireless debugging.', 'Use My devices → Wireless to pair and connect.']
      : ['Connect the unlocked phone with a USB data cable.', 'In Developer options, turn on USB debugging.', 'Approve the debugging fingerprint on the phone.'];
  el('phone-connect-steps').replaceChildren(...steps.map(step => text('li', step)));
  el('phone-connect-note').textContent = readyPhone(phone) ? phoneStatus(phone) : 'Complete the approval on the phone, then check the connection.';
  const dialog = el('phone-connect-dialog');
  if (!dialog.open) dialog.showModal();
}
el('phone-authorize').addEventListener('click', () => {
  const phone = [...usbPhones, ...wirelessPhones].find(item => phoneKey(item) === selectedPhone);
  if (phone) openPhoneAuthorization(phone);
});
el('phone-connect-check').addEventListener('click', () => run('Checking phone connection…', async () => {
  try {
    await loadPhones();
    const phones = phoneMode === 'usb' ? usbPhones : wirelessPhones;
    const same = phones.find(phone => phone.transport === dialogPhone.transport &&
      (phoneKey(phone) === phoneKey(dialogPhone) || (dialogPhone.target && phone.target === dialogPhone.target)));
    el('phone-connect-note').textContent = same ? phoneStatus(same) : 'Reopen the device from My devices after approving the phone prompt.';
  } catch (error) {
    el('phone-connect-note').textContent = typeof error === 'string' ? error : error.message;
  }
}));
el('wifi-pair').addEventListener('click', () => run('Pairing with the Android phone…', async () => {
  const address = el('wifi-pair-address').value.trim();
  const code = el('wifi-pair-code').value;
  el('wifi-pair-code').value = '';
  await invoke('pair_wireless_android', {address, code});
  el('phone-wireless-status').textContent = 'Paired. Use the connection address to connect, or refresh to see an automatically connected phone.';
  return '';
}));
el('wifi-connect').addEventListener('click', () => run('Connecting to Android Wireless debugging…', async () => {
  await invoke('connect_wireless_android', {address: el('wifi-connect-address').value.trim()});
  await loadPhones();
  el('phone-wireless-status').textContent = 'Wireless connection confirmed.';
  return '';
}));
renderPhones();

el('acquisition-transport').addEventListener('change', () => {
  el('acquisition-device').replaceChildren(new Option('Refresh to detect devices', ''));
  el('acquisition-scope-label').hidden = !el('acquisition-transport').value.startsWith('android');
  el('acquisition-status').textContent = 'Detect connected devices for this transport.';
  window.syncWorkspaceControls();
});
el('acquisition-device').addEventListener('change', () => {
  el('start-acquisition').disabled = busy || !el('acquisition-device').value;
});
function showCaptureTab(tab) {
  for (const button of document.querySelectorAll('[data-capture-tab]')) {
    const active = button.dataset.captureTab === tab;
    button.setAttribute('aria-pressed', String(active));
    el(`capture-panel-${button.dataset.captureTab}`).hidden = !active;
  }
}
for (const button of document.querySelectorAll('[data-capture-tab]')) {
  button.addEventListener('click', () => {
    showCaptureTab(button.dataset.captureTab);
    if (button.dataset.captureTab === 'device' && !el('acquisition-device').value) {
      run('Checking connected devices…', detectAcquisitionDevices);
    }
  });
}
async function detectAcquisitionDevices() {
  const list = el('acquisition-device');
  const selected = list.value;
  list.replaceChildren(new Option('Choose a connected device', ''));
  const transport = el('acquisition-transport').value;
  const devices = await invoke('list_acquisition_devices', {transport});
  const options = devices.filter(device => !phoneBinding || (device.id === phoneBinding.id && transport === phoneBinding.transport)).map(device => {
    const option = new Option([device.model?.replaceAll('_',' '), device.id, words(device.state)].filter(Boolean).join(' · '), device.id);
    option.disabled = !['device', 'attached'].includes(device.state);
    return option;
  });
  list.append(...options);
  const available = options.filter(option => !option.disabled);
  if (available.some(option => option.value === selected)) list.value = selected;
  else if (available.length === 1) list.value = available[0].value;
  el('acquisition-status').textContent = options.length
    ? 'Choose an authorized device and where to save its capture.'
    : phoneBinding ? 'Reconnect this workspace’s assigned phone.' : 'No connected device was reported by the selected transport.';
  return '';
}
el('refresh-acquisition').addEventListener('click', () => run('Detecting connected devices…', detectAcquisitionDevices));
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

el('choose-history').addEventListener('click', () => run('Opening backup history…', async () => {
  const selected = await invoke('pick_source', {slot: 'backup_history'});
  if (!selected) return '';
  const history = await invoke('backup_history');
  el('history-folder').textContent = selected.name;
  el('history-list').replaceChildren(...history.entries.map(entry => {
    const row = text('div', '', 'device-row');
    const detail = text('div', '', 'volume-label');
    detail.append(text('strong', words(entry.acquisition)), text('span', `${new Date(entry.completed_unix_ms).toLocaleString()} · ${size(entry.bytes)} · ${entry.name}`, 'muted'));
    const verify = text('button', 'Verify');
    verify.dataset.action = '';
    verify.addEventListener('click', () => run('Checking backup hashes…', async () => {
      const count = await invoke('verify_history_item', {name: entry.name});
      return `Verified ${number.format(count)} backup files.`;
    }));
    row.append(detail, verify);
    return row;
  }));
  const notes = [];
  if (!history.entries.length) notes.push('No completed captures in this folder.');
  if (history.unreadable) notes.push(`${number.format(history.unreadable)} manifests could not be read.`);
  if (history.limited) notes.push('The folder exceeded the history scan limit.');
  if (notes.length) el('history-list').append(text('p', notes.join(' '), 'muted'));
  return '';
}));

async function refreshAndroidService() {
  const transport = el('service-android-transport').value;
  const select = el('service-android-device');
  const previous = select.value;
  select.replaceChildren(new Option('Choose an authorized phone', ''));
  el('service-specs').replaceChildren();
  servicePhones = [];
  el('service-android-state').textContent = 'Checking connection…';
  try {
    const devices = await invoke('list_acquisition_devices', {transport});
    servicePhones = devices.filter(device => device.state === 'device').map(device => ({
      transport, target: device.id, name: device.model?.replaceAll('_', ' ') || device.id,
    }));
    select.append(...servicePhones.map(phone => new Option(`${phone.name} · ${phone.target}`, phone.target)));
    if (servicePhones.some(phone => phone.target === previous)) select.value = previous;
    else if (servicePhones.length === 1) select.value = servicePhones[0].target;
    const connection = transport === 'android' ? 'USB' : 'Wi-Fi';
    el('service-android-state').textContent = servicePhones.length
      ? `${number.format(servicePhones.length)} authorized ${connection} ${servicePhones.length === 1 ? 'phone' : 'phones'}`
      : transport === 'android'
        ? 'No authorized USB phones. Unlock the phone and approve its USB debugging prompt.'
        : 'No authorized Wi-Fi phones. Pair and connect the phone in My devices.';
  } catch (error) {
    el('service-android-state').textContent = 'Device detection failed. Check the ADB installation and refresh.';
    throw error;
  }
}

for (const [id, event] of [['refresh-service-android', 'click'], ['service-android-transport', 'change']]) {
  el(id).addEventListener(event, () => run('Checking Android connections…', refreshAndroidService));
}
el('service-android-device').addEventListener('change', () => {
  el('service-specs').replaceChildren();
  controls();
});
el('service-read-specs').addEventListener('click', () => run('Reading Android specifications…', async () => {
  const report = await invoke('android_details', {id:el('service-android-device').value, transport:el('service-android-transport').value});
  const properties = report.properties;
  describe('service-specs', [['Model', properties.model], ['OS', properties.android_version && `Android ${properties.android_version}`],
    ['Architecture', properties.cpu_abi], ['Display', report.display], ['Battery', report.battery_percent == null ? null : `${report.battery_percent}%`],
    ['Shared storage available', report.shared_storage ? size(report.shared_storage.available) : null]].filter(([, value]) => value != null));
}));
el('service-capture').addEventListener('click', () => {
  const phone = servicePhones.find(phone => phone.target === el('service-android-device').value);
  if (phone) capturePhoneCategory(phone, preferences.scope);
});
pick('pick-firmware', 'firmware', 'firmware-name', () => {});
el('save-firmware-plan').addEventListener('click', () => run('Checking the selected firmware image…', async () => {
  const value = name => el(`firmware-${name}`).value.trim();
  if (!value('case') || !value('model') || !value('target') || !/^[a-f0-9]{64}$/i.test(value('hash'))) {
    throw Error('Enter a case, device model, target identifier and 64-character vendor SHA-256.');
  }
  const saved = await invoke('firmware_preflight', {case:value('case'), model:value('model'), target:value('target'), sha256:value('hash')});
  return saved ? `Saved ${saved}. The plan is ready for signed review.` : '';
}));

el('detect-fastboot').addEventListener('click', () => run('Checking Android fastboot…', async () => {
  const targets = await invoke('fastboot_devices');
  el('fastboot-state').textContent = targets.length ? targets.map(target => target.id).join(' · ') : 'No Android device reported fastboot mode.';
  return '';
}));
function resetIosSelection() {
  recoveryAction = null;
  el('service-ios-device').replaceChildren(new Option('Choose a detected device', ''));
  el('change-ios-recovery').textContent = 'Request recovery transition';
  el('service-ios-state').textContent = 'Select the detection option for the current boot mode.';
}
el('detect-normal-ios').addEventListener('click', () => run('Looking for normal iOS devices…', async () => {
  resetIosSelection();
  const devices = await invoke('list_acquisition_devices', {transport: 'ios'});
  el('service-ios-device').append(...devices.map(device => new Option([device.model, device.id].filter(Boolean).join(' · '), device.id)));
  el('service-ios-state').textContent = devices.length ? 'Normal USB device detected. Confirm trust on the phone before requesting a transition.' : 'No normal iOS USB device was reported.';
  if (devices.length) {
    if (devices.length === 1) el('service-ios-device').value = devices[0].id;
    recoveryAction = 'enter';
    el('change-ios-recovery').textContent = 'Enter Recovery Mode';
  }
  return '';
}));
el('detect-ios-recovery').addEventListener('click', () => run('Reading iOS boot mode…', async () => {
  resetIosSelection();
  const device = await invoke('ios_recovery_device');
  const details = [device.product || device.model, device.mode, device.ecid].filter(Boolean).join(' · ');
  el('service-ios-state').textContent = details;
  if (device.mode === 'Recovery') {
    recoveryAction = 'exit';
    el('service-ios-device').append(new Option(details, device.ecid));
    el('service-ios-device').value = device.ecid;
    el('change-ios-recovery').textContent = 'Exit Recovery Mode';
  }
  return '';
}));
el('service-ios-device').addEventListener('change', controls);
el('service-capture-ios').addEventListener('click', () => {
  const select = el('service-ios-device');
  if (!select.value || recoveryAction !== 'enter') return;
  capturePhoneCategory({transport:'ios', target:select.value, name:select.selectedOptions[0].textContent}, 'shared');
});
el('change-ios-recovery').addEventListener('click', () => {
  const id = el('service-ios-device').value;
  const action = recoveryAction;
  if (!id || !action || !window.confirm(`Request a restart for iOS target ${id}?`)) return;
  run('Requesting iOS recovery transition…', async () => {
    await invoke('change_ios_recovery', {id, enter: action === 'enter'});
    resetIosSelection();
    el('service-ios-state').textContent = 'Transition requested. Detect the device again to confirm its mode.';
    return '';
  });
});
function openMirrorTool(transport = null) {
  run('Opening Screen mirror…', () => invoke('open_tool_window', {view:'mirror', scanSource:transport}));
}

async function loadMirrorDevices() {
  const select = el('mirror-device');
  const previous = select.value;
  const transport = el('mirror-transport').value;
  const devices = await invoke('list_acquisition_devices', {transport});
  const authorized = devices.filter(device => device.state === 'device');
  select.replaceChildren(new Option('Choose an authorized Android phone', ''));
  select.append(...authorized.map(device => new Option([device.model, device.id].filter(Boolean).join(' · '), device.id)));
  if (authorized.some(device => device.id === previous)) select.value = previous;
  else if (authorized.length === 1) select.value = authorized[0].id;
  el('mirror-state').textContent = authorized.length ? 'Camera requires Android 12 or newer. Audio requires Android 11 or newer.' : transport === 'android' ? 'No authorized USB phone detected. Check USB debugging.' : 'No paired Wi-Fi phone detected. Pair it from My devices first.';
}
el('open-mirror-nav').addEventListener('click', () => openMirrorTool());
el('open-mirror-tool').addEventListener('click', () => openMirrorTool(el('service-android-transport').value));
el('detect-mirror').addEventListener('click', () => run('Checking Android authorization…', loadMirrorDevices));
el('mirror-transport').addEventListener('change', () => {
  el('mirror-profile').value = el('mirror-transport').value === 'android_wireless' ? preferences.wifiQuality : preferences.usbQuality;
  run('Checking Android authorization…', loadMirrorDevices);
});
el('mirror-device').addEventListener('change', controls);
el('start-mirror').addEventListener('click', () => {
  const options = {profile:el('mirror-profile').value, video:el('mirror-video').value, audio:el('mirror-audio').value};
  const camera = options.video !== 'screen';
  const audio = options.audio !== 'off';
  if ((camera || audio) && !window.confirm(`Start live ${camera ? 'camera' : 'screen'} preview${audio ? (options.audio === 'microphone' ? ' with the phone microphone' : ' with device audio') : ''} on the selected phone?`)) return;
  run('Starting scrcpy…', async () => {
    await invoke('mirror_android', {id:el('mirror-device').value, transport:el('mirror-transport').value, options});
    mirrorLive = true;
    el('mirror-state').textContent = 'Live preview opened. Stop it here or close the preview window.';
  });
});
el('stop-mirror').addEventListener('click', () => run('Stopping preview…', async () => {
  await invoke('stop_mirror');
  mirrorLive = false;
  el('mirror-state').textContent = 'Preview stopped.';
}));
setInterval(async () => {
  if (busy || !mirrorLive) return;
  try {
    if (!await invoke('mirror_active')) {
      mirrorLive = false;
      el('mirror-state').textContent = 'Preview closed.';
      controls();
    }
  } catch {}
}, 1500);

pick('pick-record-file', 'artifact', 'record-file-name', () => { el('record-results').hidden = true; });
el('record-kind').addEventListener('change', () => { el('record-results').hidden = true; });
el('read-records').addEventListener('click', () => run('Reading the selected export…', async () => {
  el('record-results').hidden = true;
  const report = await invoke('inspect_export_records', {kind: el('record-kind').value});
  const fields = Object.keys(report.records[0] || {});
  const head = document.createElement('thead');
  const row = document.createElement('tr');
  row.append(...fields.map(field => text('th', words(field))));
  head.append(row);
  const body = document.createElement('tbody');
  body.append(...report.records.map(record => {
    const row = document.createElement('tr');
    row.append(...fields.map(field => text('td', record[field] == null ? '' : String(record[field]))));
    return row;
  }));
  el('record-table').replaceChildren(head, body);
  el('record-summary').textContent = `${number.format(report.records.length)} records${report.has_more ? ' · preview limited' : ''} · SHA-256 ${report.source_sha256}`;
  el('record-results').hidden = false;
  return '';
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

function clonePhone(id) {
  return clonePhones.find(phone => el(id).value === `phone:${phone.id}`);
}

function chosenCloneFolders() {
  return [...el('transfer-folders').querySelectorAll('input:checked')].map(box => box.value);
}

function renderTransferChoices() {
  const source = el('transfer-source');
  const target = el('transfer-target');
  const sourcePhone = clonePhone(source.id);
  const targetPhone = clonePhone(target.id);
  const fromExport = source.value === 'export';
  const fromFiles = source.value === 'files';
  const samePhone = sourcePhone && sourcePhone === targetPhone;
  el('transfer-folders').hidden = !sourcePhone;
  el('transfer-export-source').hidden = !fromExport;
  el('transfer-files-source').hidden = !fromFiles;
  el('transfer-picked-summary').textContent = pickedTransferCount
    ? `${number.format(pickedTransferCount)} selected ${pickedTransferCount === 1 ? 'file' : 'files'}`
    : 'Choose the files you want to copy.';
  el('transfer-selection').hidden = !fromExport || cloneList === null;
  source.closest('.transfer-device').classList.toggle('transfer-connected', Boolean(sourcePhone));
  target.closest('.transfer-device').classList.toggle('transfer-connected', Boolean(targetPhone));
  el('transfer-source-state').textContent = sourcePhone
    ? 'USB debugging authorized.'
    : fromFiles ? 'Selected files from this computer.'
      : fromExport ? 'Use a saved phone export, capture data folder, or other local files.'
      : 'Choose a connected Android phone or a local export folder.';
  el('transfer-target-state').textContent = samePhone
    ? 'Choose a different phone.'
    : targetPhone ? 'The encrypted archive will appear in Downloads.'
      : target.value === 'computer' ? 'Choose where to save the archive when you start.'
        : 'Choose a different Android phone or save the archive for later.';
  el('transfer-selection-note').textContent = sourcePhone
    ? 'For messages, contacts, and chats, choose a local export containing those files.'
    : fromFiles ? 'The archive includes the files you selected.'
      : fromExport ? 'Review files to choose individual items. The default is all compatible files in the folder.'
      : 'Choose a source to see its file options.';
  el('start-transfer').textContent = target.value === 'computer' ? 'Save encrypted archive' : 'Start transfer';
  controls();
}

async function refreshClonePhones() {
  const source = el('transfer-source');
  const target = el('transfer-target');
  const previousSource = source.value;
  const previousTarget = target.value;
  let devices = [];
  let failure = '';
  try {
    devices = await invoke('list_acquisition_devices', {transport:'android'});
  } catch (error) {
    failure = typeof error === 'string' ? error : error.message;
  }
  clonePhonesLoaded = true;
  clonePhones = devices.filter(phone => phone.state === 'device');
  const add = (select, placeholder, ...alternates) => {
    select.replaceChildren(new Option(placeholder, ''), ...alternates, ...devices.map(phone => {
      const name = phone.model?.replaceAll('_', ' ') || 'Android phone';
      const details = phone.state === 'device' ? phone.id : `${phone.id} · ${words(phone.state)}`;
      const option = new Option(`${name} · ${details}`, `phone:${phone.id}`);
      option.disabled = phone.state !== 'device';
      return option;
    }));
  };
  add(source, 'Choose a source', new Option('Local export folder', 'export'), new Option('Selected computer files', 'files'));
  add(target, 'Choose a destination', new Option('Save archive to this computer', 'computer'));
  const restore = (select, previous) => {
    if ([...select.options].some(option => option.value === previous && !option.disabled)) select.value = previous;
  };
  restore(source, previousSource);
  restore(target, previousTarget);
  if (pendingCloneSource) {
    restore(source, `phone:${pendingCloneSource}`);
    pendingCloneSource = null;
  } else if (!previousSource && clonePhones.length === 1) source.value = `phone:${clonePhones[0].id}`;
  const connected = clonePhones.length;
  const guidance = connected === 1 ? ' Connect a second phone for device-to-device transfer.' : '';
  el('transfer-phone-status').textContent = failure || (connected
    ? `${number.format(connected)} authorized USB ${connected === 1 ? 'phone' : 'phones'}.${guidance}`
    : 'Unlock the phones, approve USB debugging, then refresh. You can also use a local export folder.');
  renderTransferChoices();
}

el('refresh-transfer-phones').addEventListener('click', () => run('Detecting Android USB phones…', refreshClonePhones));
for (const id of ['transfer-source', 'transfer-target']) {
  el(id).addEventListener('change', () => {
    el('transfer-result-card').hidden = true;
    renderTransferChoices();
  });
}
el('transfer-folders').addEventListener('change', controls);
el('flip-transfer').addEventListener('click', () => {
  const source = el('transfer-source');
  const target = el('transfer-target');
  [source.value, target.value] = [target.value, source.value];
  el('transfer-result-card').hidden = true;
  renderTransferChoices();
});
pick('pick-transfer', 'transfer', 'transfer-name', () => {
  el('transfer-result-card').hidden = true;
  cloneList = null;
  cloneSelection.clear();
  renderTransferChoices();
});

function selectQuickTransfer(phone, dropped = null) {
  if (phone && (!readyPhone(phone) || phone.transport !== 'android')) return;
  run('Selecting files to transfer…', async () => {
    const count = await invoke('pick_transfer_files', {dropped});
    if (count == null) return;
    pickedTransferCount = count;
    if (phone) {
      await refreshClonePhones();
      el('transfer-target').value = clonePhones.some(device => device.id === phone.target) ? `phone:${phone.target}` : '';
      showView('transfer');
    }
    el('transfer-source').value = 'files';
    el('transfer-result-card').hidden = true;
    renderTransferChoices();
  });
}

el('pick-transfer-files').addEventListener('click', () => selectQuickTransfer(null));

const dropEvents = window.__TAURI__?.event;
if (dropEvents) {
  dropEvents.listen('tauri://drag-drop', ({payload}) => {
    const button = el('phone-drop-target');
    if (busy || el('my-devices').hidden || button.hidden || !payload?.paths?.length || !payload.position) return;
    const bounds = button.closest('.phone-device-frame').getBoundingClientRect();
    const x = payload.position.x / window.devicePixelRatio;
    const y = payload.position.y / window.devicePixelRatio;
    if (x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom) {
      selectQuickTransfer(currentPhone(), payload.paths);
    }
  }).then(() => { el('phone-drop-target').querySelector('small').hidden = false; }).catch(() => {});
}

el('start-transfer').addEventListener('click', () => {
  const target = clonePhone('transfer-target');
  const phone = clonePhone('transfer-source');
  const from = phone
    ? {kind:'android', id:phone.id, folders:chosenCloneFolders()}
    : el('transfer-source').value === 'files' ? {kind:'files'} : {kind:'export', selection:cloneList === null ? null : [...cloneSelection]};
  run(target ? 'Creating and transferring the encrypted archive…' : 'Preparing the encrypted archive…', async () => {
    const password = el('password').value;
    if (!/^[ -~]{12,128}$/.test(password)) throw Error('Enter a passphrase of 12–128 printable ASCII characters.');
    if (password !== el('confirm-password').value) throw Error('The passphrases do not match.');
    if (target && !window.confirm(`Send an encrypted archive to ${target.model?.replaceAll('_', ' ') || 'Android phone'} (${target.id})?`)) return;
    el('password').value = '';
    el('confirm-password').value = '';
    el('transfer-result-card').hidden = true;
    const result = await invoke('create_transfer', {source:from, target:target?.id || null, password});
    if (!result) return;
    const missing = result.unavailable_folders.length ? ` Unavailable folders: ${result.unavailable_folders.join(', ')}.` : '';
    el('transfer-result-title').textContent = result.delivery ? 'Archive delivered and verified' : 'Archive saved';
    el('transfer-result').textContent = `${number.format(result.backup.files)} files · ${size(result.backup.archive_bytes)} encrypted archive.${missing}`;
    el('transfer-result-path').textContent = result.delivery ? result.path.replace(/^\/sdcard\/Download\//, 'Downloads / ') : result.path;
    el('transfer-result-next').textContent = result.delivery
      ? 'On the target, open the OmniRecovery companion, choose this archive in Downloads, and select Verify and restore files.'
      : 'Copy the archive to the receiving Android phone and open it in the OmniRecovery companion.';
    el('transfer-result-card').hidden = false;
  });
});

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
el('sanitize-phone').addEventListener('change', controls);
el('save-phone-wipe-plan').addEventListener('click', () => run('Preparing a phone wipe plan…', async () => {
  const caseName = el('sanitize-phone-case').value.trim();
  if (!caseName) throw Error('Enter a case reference.');
  const saved = await invoke('phone_wipe_plan', {case:caseName, id:el('sanitize-phone').value});
  return saved ? `Saved ${saved}. The phone was not changed.` : '';
}));
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
  if (detail === 'my-devices') {
    renderPhones();
    if (phoneBinding ? !boundInventoryLoaded : !phonesLoaded && phoneMode === 'usb') {
      run('Checking connected phones…', loadPhones);
    }
  } else {
    el('phone-sidebar').querySelectorAll('[aria-current]').forEach(item => item.removeAttribute('aria-current'));
    el('phone-sidebar').querySelectorAll('.phone-sidebar-category').forEach(item => item.remove());
    el('wifi-pair-code').value = '';
  }
  if (detail === 'transfer' && (!clonePhonesLoaded || pendingCloneSource)) run('Detecting Android USB phones…', refreshClonePhones);
  if (detail === 'service-android') run('Checking Android connections…', refreshAndroidService);
  if (detail === 'mirror' && !el('mirror-device').value) run('Checking Android authorization…', loadMirrorDevices);
  if ((detail === 'devices' || detail === 'sanitize') && !deviceList) run('Reading connected devices…', updateDevices);
  if (detail === 'capture') run('Loading capture workspace…', async () => {
    await refreshCaptures();
    if (!el('capture-panel-device').hidden && !el('acquisition-device').value &&
        (phoneBinding || el('acquisition-transport').value === 'android')) {
      await detectAcquisitionDevices();
    }
  });
  if (detail === 'logs') run('Loading scan logs…', refreshLogs);
});

invoke('media_saving_available').then(available => {
  canCopy = available;
  el('clone-evidence').hidden = !available;
  el('clone-image').hidden = !available || captureSource() !== 'capture_image';
}).catch(() => {});
controls();

invoke('tool_startup').then(tool => {
  if (!tool) return;
  if (tool.phone) {
    phoneBinding = tool.phone;
    phoneMode = phoneBinding.transport === 'android_wireless' ? 'wireless' : 'usb';
    el('acquisition-transport').value = phoneBinding.transport;
    el('acquisition-scope-label').hidden = phoneBinding.transport === 'ios';
  } else if (tool.view === 'mirror' && tool.scan_source) {
    el('mirror-transport').value = tool.scan_source;
    el('mirror-profile').value = tool.scan_source === 'android_wireless' ? preferences.wifiQuality : preferences.usbQuality;
  } else if (tool.scan_source) {
    el('scan-source').value = tool.scan_source;
    scanSourceChanged();
  }
  showView(tool.view);
}).catch(error => status(typeof error === 'string' ? error : error.message, 'error'));
