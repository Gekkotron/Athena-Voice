'use strict';

const T = {
  en: {
    save: 'Save', skills: 'Skills',
    enabled: 'enabled', disabled: 'disabled', loaded: 'loaded', not_loaded: 'not loaded',
    enable: 'Enable', disable: 'Disable', back: '← Back', add_row: 'Add row', remove: 'Remove',
    saved: 'Saved.', reload_failed: 'Saved, but reload failed: ',
    secret_set: 'A value is stored. Leave blank to keep it.',
    upload_title: 'Install a skill', upload_help: 'Drop a .wasm file or pick a bundled skill.',
    install: 'Install', no_settings: 'This skill has no settings.',
    needs_config: 'needs config', key: 'Key', value: 'Value',
    test_connection: 'Test connection', testing: 'Testing…',
    jeedom_ok: 'Jeedom reachable — API key valid', jeedom_unauthorized: 'Invalid API key',
    jeedom_unreachable: 'Jeedom unreachable — check the URL', jeedom_bad_response: 'Unexpected reply — is this a Jeedom URL?',
    jeedom_unconfigured: 'Save the URL and API key first',
    discover: 'Discover sensors', discovering: 'Scanning…',
    add_selection: 'Add selection', nothing_discovered: 'No readable commands found',
    read: 'Read', resync: 'Re-sync', you_can_say: 'You can say…',
    duplicate_phrase: 'duplicate — another sensor answers the same phrase',
    matched_as: 'matched as', apply: 'Apply', gone_from_jeedom: 'gone from Jeedom',
    no_phrases: 'no phrases — save sensors first',
    test_console: 'Test console', test_console_help: 'Send a text command to the assistant — test tool, nothing is stored server-side.',
    send: 'Send', sending: 'Sending…', network_error: 'network error',
    action_onoff: 'on/off device',
    action_shutter: 'shutter', with_stop: '+ stop', with_position: '+ position',
    section_connection: 'Connection', section_sensors: 'Sensors',
    section_actions: 'On/off devices', section_shutters: 'Shutters',
    section_discovery: 'Discovery',
    filter_search: 'Search a device…', filter_all_rooms: 'All rooms',
    filter_all_types: 'All types', type_sensor: 'sensor',
    no_filter_match: 'No device matches the filters.',
    devices_shown: (n, total) => `${n} / ${total} devices`,
  },
  fr: {
    save: 'Enregistrer', skills: 'Compétences',
    enabled: 'activée', disabled: 'désactivée', loaded: 'chargée', not_loaded: 'non chargée',
    enable: 'Activer', disable: 'Désactiver', back: '← Retour', add_row: 'Ajouter', remove: 'Retirer',
    saved: 'Enregistré.', reload_failed: 'Enregistré, mais rechargement échoué : ',
    secret_set: 'Une valeur est enregistrée. Laissez vide pour la conserver.',
    upload_title: 'Installer une compétence', upload_help: 'Déposez un fichier .wasm ou choisissez une compétence fournie.',
    install: 'Installer', no_settings: 'Cette compétence n’a aucun réglage.',
    needs_config: 'à configurer', key: 'Clé', value: 'Valeur',
    test_connection: 'Tester la connexion', testing: 'Test en cours…',
    jeedom_ok: 'Jeedom joignable — clé API valide', jeedom_unauthorized: 'Clé API invalide',
    jeedom_unreachable: 'Jeedom injoignable — vérifiez l’URL', jeedom_bad_response: 'Réponse inattendue — est-ce bien une URL Jeedom ?',
    jeedom_unconfigured: 'Enregistrez d’abord l’URL et la clé API',
    discover: 'Découvrir les capteurs', discovering: 'Analyse en cours…',
    add_selection: 'Ajouter la sélection', nothing_discovered: 'Aucune commande lisible trouvée',
    read: 'Lire', resync: 'Re-synchroniser', you_can_say: 'Vous pouvez dire…',
    duplicate_phrase: 'doublon — un autre capteur répond à la même phrase',
    matched_as: 'entendu comme', apply: 'Appliquer', gone_from_jeedom: 'disparu de Jeedom',
    no_phrases: 'aucune phrase — enregistrez des capteurs',
    test_console: 'Console de test', test_console_help: 'Envoyer une commande texte à l’assistant — outil de test, rien n’est stocké côté serveur.',
    send: 'Envoyer', sending: 'Envoi…', network_error: 'erreur réseau',
    action_onoff: 'appareil on/off',
    action_shutter: 'volet', with_stop: '+ stop', with_position: '+ position',
    section_connection: 'Connexion', section_sensors: 'Capteurs',
    section_actions: 'Appareils on/off', section_shutters: 'Volets',
    section_discovery: 'Découverte',
    filter_search: 'Rechercher un appareil…', filter_all_rooms: 'Toutes les pièces',
    filter_all_types: 'Tous les types', type_sensor: 'capteur',
    no_filter_match: 'Aucun appareil ne correspond aux filtres.',
    devices_shown: (n, total) => `${n} / ${total} appareils`,
  },
};
const lang = (navigator.language || 'en').startsWith('fr') ? 'fr' : 'en';
const t = (k) => T[lang][k] || k;

// Human column headers for list editors; unknown keys fall back to the raw
// config key so non-jeedom skills stay readable.
const COL_LABELS = {
  en: {
    name: 'Spoken name', id: 'Cmd', unit: 'Unit', room: 'Room', prefix: 'Connector',
    kind: 'Type', on_label: 'ON label', off_label: 'OFF label',
    on_id: 'ON cmd', off_id: 'OFF cmd', confirm: 'Confirm',
    up_id: 'Up cmd', down_id: 'Down cmd', stop_id: 'Stop cmd', slider_id: 'Position cmd',
  },
  fr: {
    name: 'Nom parlé', id: 'Cmd', unit: 'Unité', room: 'Pièce', prefix: 'Liaison',
    kind: 'Type', on_label: 'Libellé ON', off_label: 'Libellé OFF',
    on_id: 'Cmd ON', off_id: 'Cmd OFF', confirm: 'Confirmation',
    up_id: 'Cmd monter', down_id: 'Cmd descendre', stop_id: 'Cmd stop', slider_id: 'Cmd position',
  },
};
const colLabel = (k) => COL_LABELS[lang][k] || k;

// French article lookup for composed sensor names; unknown rooms get no
// article ("température salon") — still fuzzy-matchable and editable.
const FR_ROOM_ARTICLES = {
  salon: 'du', bureau: 'du', garage: 'du', couloir: 'du', grenier: 'du', jardin: 'du',
  chambre: 'de la', cuisine: 'de la', terrasse: 'de la', cave: 'de la',
  'salle de bain': 'de la', 'salle à manger': 'de la', buanderie: 'de la',
};
// Command names that describe no equipment on their own ("état", "valeur")
// — real Jeedom sensors are often equipment "Porte" + command "État", and
// composing from the command alone would speak "état du garage" instead of
// "porte du garage". When the command name is one of these, compose from
// the equipment name instead.
const GENERIC_CMD_NAMES = ['état', 'etat', 'statut', 'status', 'state', 'valeur', 'value', 'ouverture'];
// Article guess for a room, as the de-form connector stored in the prefix
// column. Straight apostrophes only — the skill's parse-time cleaning
// normalizes « ’ » to « ' », and generated names should start clean.
function guessRoomPrefix(room) {
  const r = (room || '').toLowerCase();
  if (!r) return '';
  if (/^[aeéèiouy]/.test(r)) return "de l'";
  return FR_ROOM_ARTICLES[r] || '';
}

// "d'alicia" / "du salon" / bare room when no prefix — the one spacing rule
// shared by name composition and the prefix-edit suffix swap.
function composedRoomSuffix(prefix, room) {
  if (!room) return '';
  if (!prefix) return room;
  return /['’]$/.test(prefix) ? `${prefix}${room}` : `${prefix} ${room}`;
}

function composeSensorName(cmdName, eqName, room, prefix) {
  const isGeneric = GENERIC_CMD_NAMES.includes(cmdName.toLowerCase());
  const cmd = (isGeneric ? eqName : cmdName).toLowerCase();
  if (!room) return cmd;
  const r = room.toLowerCase();
  const p = prefix !== undefined ? prefix : guessRoomPrefix(r);
  return `${cmd} ${composedRoomSuffix(p, r)}`;
}

// Prefix edit: recompose the name ONLY when it still ends with the old
// prefix+room suffix (never customized); a hand-edited name always wins.
function swapNameSuffix(name, oldPrefix, newPrefix, room) {
  const r = (room || '').toLowerCase();
  const oldSuffix = composedRoomSuffix(oldPrefix, r);
  if (!r || !oldSuffix || !name.endsWith(oldSuffix)) return name;
  return name.slice(0, name.length - oldSuffix.length) + composedRoomSuffix(newPrefix, r);
}

// Mirror of the matcher's `normalize_literal` (runtime intent/engine.rs):
// lowercase; keep letters/digits of any script, apostrophes, and hyphens;
// collapse every dropped char or whitespace run into a single space. This is
// the form the matcher actually compares against speech, so it drives both
// the symbols chip ("entendu comme …") and duplicate detection.
function normalizeLiteral(s) {
  let out = '';
  let pending = false;
  for (const c of String(s).toLowerCase()) {
    if (/[\p{L}\p{N}'-]/u.test(c)) {
      if (pending && out) out += ' ';
      out += c;
      pending = false;
    } else {
      pending = true;
    }
  }
  return out;
}

// Normalized phrases that appear under MORE THAN ONE per-device intent
// (jeedom.read.{id}, jeedom.turn_on.{id}, jeedom.shutter_open.{id}) — name
// collisions like six sensors all called "température".
// groups: intent -> {locale -> [phrases]}.
function duplicatePhrases(groups) {
  const firstIntent = new Map();
  const dupes = new Set();
  for (const [intent, locales] of Object.entries(groups)) {
    if (!/^jeedom\.(read|turn_on|shutter_open)\.\d+$/.test(intent)) continue;
    for (const phrases of Object.values(locales)) {
      for (const p of phrases) {
        const n = normalizeLiteral(p);
        if (!firstIntent.has(n)) firstIntent.set(n, intent);
        else if (firstIntent.get(n) !== intent) dupes.add(n);
      }
    }
  }
  return dupes;
}

const app = document.getElementById('app');

async function api(path, opts = {}) {
  let res;
  try {
    res = await fetch(path, opts);
  } catch {
    document.getElementById('status').textContent = 'API unreachable';
    throw new Error('network error');
  }
  if (!res.ok && res.status >= 500) {
    document.getElementById('status').textContent = `API error ${res.status}`;
  }
  return res;
}

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'onclick') node.addEventListener('click', v);
    else if (k === 'text') node.textContent = v;
    else node.setAttribute(k, v);
  }
  node.append(...children);
  return node;
}

// --- Test console (test tool): one-shot text command over the satellite
// path, with a client-side history of the last commands. ---

const TEST_HISTORY_KEY = 'athena-test-history';
const TEST_HISTORY_MAX = 20;

function loadTestHistory() {
  try {
    const h = JSON.parse(localStorage.getItem(TEST_HISTORY_KEY));
    return Array.isArray(h) ? h.filter((x) => typeof x === 'string') : [];
  } catch { return []; }
}

// Most recent first, deduped, capped.
function pushTestHistory(cmd) {
  const h = [cmd, ...loadTestHistory().filter((c) => c !== cmd)].slice(0, TEST_HISTORY_MAX);
  localStorage.setItem(TEST_HISTORY_KEY, JSON.stringify(h));
}

function testConsoleCard() {
  const input = el('input', { type: 'text', autocomplete: 'off', class: 'test-input' });
  const btn = el('button', { text: t('send') });
  const out = el('p', { class: 'test-answer' });
  const historyList = el('div', { class: 'test-history' });
  let histIdx = -1; // -1 = editing a fresh draft
  let draft = '';

  const renderHistory = () => {
    historyList.replaceChildren(...loadTestHistory().map((cmd) =>
      el('span', {
        class: 'badge test-history-item', text: cmd,
        onclick: () => { input.value = cmd; histIdx = -1; input.focus(); },
      })));
  };

  const submit = async () => {
    const text = input.value.trim();
    if (!text || input.disabled) return;
    input.disabled = true; btn.disabled = true;
    out.textContent = t('sending');
    try {
      const res = await api('/api/test-command', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text, locale: lang }),
      });
      const body = await res.json().catch(() => ({}));
      out.textContent = res.ok ? (body.answer || '—') : (body.error || `HTTP ${res.status}`);
    } catch { out.textContent = t('network_error'); }
    input.disabled = false; btn.disabled = false;
    pushTestHistory(text); histIdx = -1; draft = '';
    renderHistory();
    input.value = ''; input.focus();
  };

  // Shell-style recall: ArrowUp walks back through history, ArrowDown
  // forward and finally back to the unsent draft.
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); submit(); return; }
    const h = loadTestHistory();
    if (e.key === 'ArrowUp') {
      if (!h.length) return;
      e.preventDefault();
      if (histIdx === -1) draft = input.value;
      histIdx = Math.min(histIdx + 1, h.length - 1);
      input.value = h[histIdx];
    } else if (e.key === 'ArrowDown') {
      if (histIdx === -1) return;
      e.preventDefault();
      histIdx -= 1;
      input.value = histIdx === -1 ? draft : h[histIdx];
    }
  });
  btn.addEventListener('click', submit);
  renderHistory();

  return el('section', { class: 'card' },
    el('h2', { text: t('test_console') }),
    el('p', { class: 'test-help', text: t('test_console_help') }),
    el('div', { class: 'test-row' }, input, btn),
    out,
    historyList,
  );
}

async function renderList() {
  let skills;
  try {
    skills = await (await api('/api/skills')).json();
  } catch { return; }
  const status = await (await api('/api/status')).json();
  document.getElementById('status').textContent =
    `v${status.version} — ${status.skills_loaded} ${t('loaded')}`;

  const list = el('section', { class: 'card' }, el('h2', { text: t('skills') }));
  for (const s of skills) {
    const badges = [
      el('span', { class: `badge ${s.enabled ? 'ok' : 'off'}`, text: s.enabled ? t('enabled') : t('disabled') }),
      el('span', { class: `badge ${s.loaded ? 'ok' : ''}`, text: s.loaded ? t('loaded') : t('not_loaded') }),
    ];
    if (s.schema && s.schema.fields.some((f) => f.required) && Object.keys(s.config).length === 0) {
      badges.push(el('span', { class: 'badge off', text: t('needs_config') }));
    }
    list.append(el('div', { class: 'skill-row' },
      el('span', { class: 'name', text: s.name, onclick: () => renderDetail(s) }),
      ...badges,
      el('button', {
        class: s.enabled ? 'danger' : '',
        text: s.enabled ? t('disable') : t('enable'),
        onclick: async () => {
          await api(`/api/skills/${s.name}/${s.enabled ? 'disable' : 'enable'}`, { method: 'POST' });
          renderList();
        },
      }),
    ));
  }
  app.replaceChildren(list, await uploadCard(), testConsoleCard());
}

function fieldInput(f, current, opts) {
  if (f.type === 'list') return listEditor(f, current, opts);
  const type = f.type === 'secret' ? 'password' : f.type === 'number' ? 'number' : 'text';
  const input = el('input', { type, id: `f-${f.key}`, autocomplete: 'off' });
  if (current && current.kind === 'plain') input.value = current.value;
  const wrap = el('div', {}, el('label', { for: `f-${f.key}`, text: f.label || f.key }), input);
  if (f.type === 'secret' && current && current.set) {
    wrap.append(el('p', { class: 'secret-set', text: t('secret_set') }));
  }
  if (f.help) wrap.append(el('p', { class: 'help', text: f.help }));
  return wrap;
}

// Optional column hooks so a skill view can shape the table without the
// editor knowing that skill:
//   opts.selects:     { colKey: [choices] } — a <select> instead of an input
//   opts.enabledWhen: { colKey: (row) => bool } — disable the cell when false
//   opts.rowActions:  (row, i) => [Node…] — extra cells per row
//   opts.rowDetail:   (row, i) => Node|null — full-width line under the row
//   opts.onEdit:      () => void — any user edit to any cell
function listEditor(f, current, opts = {}) {
  let rows = [];
  try { rows = current && current.kind === 'plain' ? JSON.parse(current.value) : []; } catch {}
  const table = el('table', { 'data-list': f.key });
  const edited = () => { if (opts.onEdit) opts.onEdit(); };
  const render = () => {
    const actionCols = opts.rowActions ? 1 : 0;
    table.replaceChildren(
      el('tr', {},
        ...f.item_fields.map((c) => el('th', { text: colLabel(c.key) })),
        ...(opts.rowActions ? [el('th')] : []),
        el('th')),
      ...rows.flatMap((row, i) => {
        const tds = f.item_fields.map((c) => {
          const choices = opts.selects && opts.selects[c.key];
          let cell;
          if (choices) {
            cell = el('select', {}, ...choices.map((v) => el('option', { value: v, text: v })));
            cell.value = choices.includes(row[c.key]) ? row[c.key] : choices[0];
            // A select change can re-enable/disable sibling cells — re-render.
            cell.onchange = () => { row[c.key] = cell.value; edited(); render(); };
          } else if (c.type === 'bool') {
            cell = el('input', { type: 'checkbox' });
            cell.checked = row[c.key] === true;
            cell.onchange = () => { row[c.key] = cell.checked; edited(); };
          } else {
            cell = el('input', { type: c.type === 'number' ? 'number' : 'text' });
            cell.value = row[c.key] ?? '';
            // `change` fires after `oninput` already mutated the row, so the
            // pre-edit value is snapshotted at focus for onCellChange.
            let before;
            cell.onfocus = () => { before = row[c.key]; };
            cell.oninput = () => { row[c.key] = c.type === 'number' ? Number(cell.value) : cell.value; edited(); };
            cell.onchange = () => { if (opts.onCellChange) opts.onCellChange(c.key, row, before); };
          }
          const enabled = opts.enabledWhen && opts.enabledWhen[c.key];
          if (enabled) cell.disabled = !enabled(row);
          return el('td', {}, cell);
        });
        if (opts.rowActions) tds.push(el('td', {}, ...opts.rowActions(row, i)));
        tds.push(el('td', {}, el('button', {
          class: 'quiet', text: t('remove'),
          onclick: () => { rows.splice(i, 1); edited(); render(); },
        })));
        const trs = [el('tr', {}, ...tds)];
        const detail = opts.rowDetail && opts.rowDetail(row, i);
        if (detail) {
          const td = el('td', {}, detail);
          td.setAttribute('colspan', String(f.item_fields.length + 1 + actionCols));
          trs.push(el('tr', { class: 'row-detail' }, td));
        }
        return trs;
      }),
    );
  };
  render();
  table.getRows = () => rows;
  table.addRows = (newRows) => { rows.push(...newRows); edited(); render(); };
  table.rerender = render;
  return el('div', {},
    el('label', { text: f.label || f.key }), table,
    el('button', { class: 'quiet', text: t('add_row'), onclick: () => { rows.push({}); edited(); render(); } }),
    f.help ? el('p', { class: 'help', text: f.help }) : '',
  );
}

async function renderDetail(skill) {
  const card = el('section', { class: 'card' },
    el('button', { class: 'quiet', text: t('back'), onclick: renderList }),
    el('h2', { text: skill.name }),
  );
  const msg = el('p', { class: 'error' });
  const fields = skill.schema ? skill.schema.fields
    : Object.keys(skill.config).map((k) => ({ key: k, label: k, type: 'string', item_fields: [] }));
  if (skill.schema && fields.length === 0) {
    card.append(el('p', { class: 'help', text: t('no_settings') }));
  }
  // --- jeedom sensor-table state: live reads, phrase hints, re-sync diffs.
  // One object so rowDetail/rowActions closures and the buttons below share
  // it. `findSensorsTable` closes over `widgets` (declared later) but is
  // only ever called on user clicks, long after initialization. ---
  const jd = skill.name === 'jeedom' ? {
    reads: {},            // sensor id -> {status, value}
    reading: false,       // one in-flight read at a time
    phraseGroups: {},     // intent -> {locale -> [phrases]}
    duplicates: new Set(),
    expanded: new Set(),  // sensor ids with the full phrase list shown
    stale: false,         // edits since the last phrases fetch
    diffs: {},            // sensor id -> [{field, value}]
    missing: new Set(),   // sensor ids absent from the last re-sync
  } : null;
  const findSensorsTable = () =>
    widgets.find(([f]) => f.key === 'sensors')?.[1].querySelector('table');
  const findActionsTable = () =>
    widgets.find(([f]) => f.key === 'actions')?.[1].querySelector('table');
  const findShuttersTable = () =>
    widgets.find(([f]) => f.key === 'shutters')?.[1].querySelector('table');
  const readCell = (row) => {
    const id = Number(row.id);
    const r = jd.reads[id];
    const out = el('span', { class: r && r.status === 'ok' ? 'read-ok' : 'read-err' });
    if (r) out.textContent = r.status === 'ok'
      ? `${r.value}${row.unit ? ` ${row.unit}` : ''}`
      : t(`jeedom_${r.status}`);
    return [el('button', {
      class: 'quiet', text: t('read'),
      onclick: async () => {
        if (jd.reading || !Number.isFinite(id)) return;
        jd.reading = true;
        try {
          const res = await api(`/api/skills/jeedom/read/${id}`, { method: 'POST' });
          jd.reads[id] = res.ok ? await res.json() : { status: 'bad_response' };
        } finally { jd.reading = false; }
        findSensorsTable()?.rerender();
      },
    }), out];
  };
  const sensorOpts = jd ? {
    selects: { kind: ['numeric', 'binary'] },
    enabledWhen: {
      on_label: (row) => row.kind === 'binary',
      off_label: (row) => row.kind === 'binary',
    },
    rowActions: readCell,
    rowDetail: (row) => sensorDetail(row),
    onEdit: () => { jd.stale = true; findSensorsTable()?.classList.add('stale'); },
    onCellChange: (key, row, oldValue) => {
      if (key !== 'prefix') return;
      // Normalize what was typed before it drives anything: straight
      // apostrophe (the skill does the same at parse time) and no stray
      // spaces — a trailing space would compose a double-space name.
      const typed = String(row.prefix || '');
      row.prefix = typed.replace(/’/g, "'").trim();
      const swapped = swapNameSuffix(
        String(row.name || ''), String(oldValue || ''),
        row.prefix, String(row.room || ''),
      );
      const nameChanged = swapped !== row.name;
      if (nameChanged) row.name = swapped;
      if (nameChanged || row.prefix !== typed) findSensorsTable()?.rerender();
    },
  } : undefined;
  const actionOpts = jd ? {
    onEdit: () => { jd.stale = true; findActionsTable()?.classList.add('stale'); },
    rowDetail: (row) => {
      const locales = jd.phraseGroups[`jeedom.turn_on.${Number(row.on_id)}`];
      const phrases = locales ? (locales[lang] || Object.values(locales)[0] || []) : [];
      if (!phrases.length) return null;
      return el('span', { class: 'hint' },
        el('span', { text: `${t('you_can_say')} ${phrases.slice(0, 2).map((p) => `« ${p} »`).join(', ')}` }));
    },
  } : undefined;
  const shutterOpts = jd ? {
    onEdit: () => { jd.stale = true; findShuttersTable()?.classList.add('stale'); },
    rowDetail: (row) => {
      const locales = jd.phraseGroups[`jeedom.shutter_open.${Number(row.up_id)}`];
      const phrases = locales ? (locales[lang] || Object.values(locales)[0] || []) : [];
      if (!phrases.length) return null;
      return el('span', { class: 'hint' },
        el('span', { text: `${t('you_can_say')} ${phrases.slice(0, 2).map((p) => `« ${p} »`).join(', ')}` }));
    },
    onCellChange: (key, row, oldValue) => {
      if (key !== 'prefix') return;
      const typed = String(row.prefix || '');
      row.prefix = typed.replace(/’/g, "'").trim();
      const swapped = swapNameSuffix(
        String(row.name || ''), String(oldValue || ''),
        row.prefix, String(row.room || ''),
      );
      const nameChanged = swapped !== row.name;
      if (nameChanged) row.name = swapped;
      if (nameChanged || row.prefix !== typed) findShuttersTable()?.rerender();
    },
  } : undefined;
  const pmsg = el('p', { class: 'help' });
  async function refreshPhrases() {
    if (!jd) return;
    let body;
    try { body = await (await api('/api/skills/jeedom/phrases')).json(); } catch { return; }
    jd.phraseGroups = {};
    for (const p of body.phrases) {
      (jd.phraseGroups[p.intent] ??= {})[p.locale] = p.phrases;
    }
    jd.duplicates = duplicatePhrases(jd.phraseGroups);
    jd.stale = false;
    const table = findSensorsTable();
    if (table) { table.classList.remove('stale'); table.rerender(); }
    const atable = findActionsTable();
    if (atable) { atable.classList.remove('stale'); atable.rerender(); }
    const stable = findShuttersTable();
    if (stable) { stable.classList.remove('stale'); stable.rerender(); }
    const anySensorGroup = Object.keys(jd.phraseGroups).some((k) => /^jeedom\.read\.\d+$/.test(k));
    pmsg.textContent = anySensorGroup ? '' : t('no_phrases');
  }
  function sensorDetail(row) {
    const id = Number(row.id);
    const bits = [];
    // Re-sync outcome (filled by the Re-sync button): per-field apply chips
    // and the gone-from-Jeedom badge. The row itself is kept — removal
    // stays the user's explicit choice.
    if (jd.missing.has(id)) bits.push(el('span', { class: 'chip warn', text: t('gone_from_jeedom') }));
    for (const d of jd.diffs[id] || []) {
      bits.push(el('span', { class: 'chip sync' },
        el('span', { text: `Jeedom: ${d.field} = ${d.value === '' ? '—' : d.value} ` }),
        el('button', {
          class: 'quiet', text: t('apply'),
          onclick: () => {
            row[d.field] = d.value;
            jd.diffs[id] = (jd.diffs[id] || []).filter((x) => x !== d);
            jd.stale = true;
            const table = findSensorsTable();
            if (table) { table.classList.add('stale'); table.rerender(); }
          },
        }),
      ));
    }
    // Symbols chip: the stored name/room carries characters the matcher
    // strips — show the form actually compared against speech.
    for (const key of ['name', 'room']) {
      const v = String(row[key] || '');
      if (v && normalizeLiteral(v) !== v.toLowerCase()) {
        bits.push(el('span', { class: 'chip warn', text: `${t('matched_as')} « ${normalizeLiteral(v)} »` }));
        break;
      }
    }
    // "Vous pouvez dire…" — what the SAVED config generates for this sensor,
    // in the UI's language (fall back to any locale that has phrases).
    const locales = jd.phraseGroups[`jeedom.read.${id}`];
    const phrases = locales ? (locales[lang] || Object.values(locales)[0] || []) : [];
    if (phrases.length) {
      if (phrases.some((p) => jd.duplicates.has(normalizeLiteral(p)))) {
        bits.push(el('span', { class: 'chip warn', text: t('duplicate_phrase') }));
      }
      const shown = jd.expanded.has(id) ? phrases : phrases.slice(0, 2);
      const line = el('span', { class: 'hint' },
        el('span', { text: `${t('you_can_say')} ${shown.map((p) => `« ${p} »`).join(', ')}` }),
      );
      if (phrases.length > shown.length) {
        line.append(el('button', {
          class: 'quiet', text: ` +${phrases.length - shown.length}`,
          onclick: () => { jd.expanded.add(id); findSensorsTable()?.rerender(); },
        }));
      }
      bits.push(line);
    }
    if (!bits.length) return null;
    return el('span', {}, ...bits);
  }
  // Jeedom's page is grouped into titled sections; other skills keep the
  // flat field list.
  const sections = {};
  const sectionFor = (key) =>
    key === 'sensors' ? 'sensors' : key === 'actions' ? 'actions'
      : key === 'shutters' ? 'shutters' : 'connection';
  const section = (key) => {
    if (!sections[key]) {
      sections[key] = el('div', { class: 'section' }, el('h3', { text: t(`section_${key}`) }));
    }
    return sections[key];
  };
  const widgets = fields.map((f) => {
    const w = fieldInput(f, skill.config[f.key],
      f.key === 'sensors' ? sensorOpts : f.key === 'actions' ? actionOpts
        : f.key === 'shutters' ? shutterOpts : undefined);
    if (jd) section(sectionFor(f.key)).append(w); else card.append(w);
    return [f, w];
  });
  if (jd) {
    for (const key of ['connection', 'sensors', 'actions', 'shutters']) {
      if (sections[key]) card.append(sections[key]);
    }
  }
  if (skill.name === 'jeedom') {
    const cmsg = el('p', { class: 'help' });
    const jmsg = el('p', { class: 'help' });
    const tree = el('div');
    section('connection').append(
      el('button', {
        class: 'quiet', text: t('test_connection'),
        onclick: async () => {
          cmsg.textContent = t('testing');
          const body = await (await api('/api/skills/jeedom/test', { method: 'POST' })).json();
          cmsg.textContent = body.status === 'ok'
            ? t('jeedom_ok')
            : t(`jeedom_${body.status}`);
        },
      }),
      cmsg,
    );
    section('discovery').append(
      el('button', {
        class: 'quiet', text: t('discover'),
        onclick: async () => {
          jmsg.textContent = t('discovering');
          const body = await (await api('/api/skills/jeedom/discover', { method: 'POST' })).json();
          if (body.status !== 'ok') { jmsg.textContent = t(`jeedom_${body.status}`); return; }
          jmsg.textContent = '';
          renderDiscoveryTree(tree, body.rooms, findSensorsTable(), findActionsTable(), findShuttersTable());
        },
      }),
      el('button', {
        class: 'quiet', text: t('resync'),
        onclick: async () => {
          jmsg.textContent = t('discovering');
          const body = await (await api('/api/skills/jeedom/discover', { method: 'POST' })).json();
          if (body.status !== 'ok') { jmsg.textContent = t(`jeedom_${body.status}`); return; }
          jmsg.textContent = '';
          const table = findSensorsTable();
          // What discovery would store today, per command id — the same
          // composition the "add selection" flow uses, so diffs compare
          // stored values against exactly what a fresh add would write.
          const fresh = new Map();
          for (const room of body.rooms) {
            for (const eq of room.equipments) {
              for (const cmd of eq.cmds) {
                fresh.set(cmd.id, {
                  cmdName: cmd.name, eqName: eq.name, roomName: room.name,
                  room: (room.name || '').toLowerCase(),
                  unit: cmd.unit || '',
                  kind: cmd.subtype === 'binary' ? 'binary' : 'numeric',
                  on_label: cmd.on_label || '',
                  off_label: cmd.off_label || '',
                });
              }
            }
          }
          jd.diffs = {};
          jd.missing = new Set();
          for (const row of table?.getRows() || []) {
            const id = Number(row.id);
            const disc = fresh.get(id);
            if (!disc) { jd.missing.add(id); continue; }
            // The compared name honors the ROW's stored prefix — the user's
            // correction, not a Jeedom fact — and prefix itself is never
            // diffed, so re-sync can't offer to revert « d' » to « de l' ».
            const wanted = {
              name: composeSensorName(disc.cmdName, disc.eqName, disc.roomName,
                row.prefix ? String(row.prefix) : undefined),
              room: disc.room, unit: disc.unit, kind: disc.kind,
              on_label: disc.on_label, off_label: disc.off_label,
            };
            jd.diffs[id] = Object.entries(wanted)
              .filter(([field, value]) => {
                // Stored kind may be '' — the skill treats that as numeric.
                const stored = field === 'kind' ? (row.kind || 'numeric') : String(row[field] ?? '');
                return stored !== String(value);
              })
              .map(([field, value]) => ({ field, value }));
          }
          // Sensors NOT in the table go through the existing tree flow.
          renderDiscoveryTree(tree, body.rooms, table, findActionsTable(), findShuttersTable());
          table?.rerender();
        },
      }),
      jmsg, pmsg, tree,
    );
    card.append(section('discovery'));
    refreshPhrases();
  }
  card.append(el('button', {
    text: t('save'),
    onclick: async () => {
      const values = {};
      for (const [f, w] of widgets) {
        if (f.type === 'list') {
          values[f.key] = JSON.stringify(w.querySelector('table').getRows());
        } else {
          const v = w.querySelector('input').value;
          if (f.type === 'secret' && v === '') continue; // blank secret = unchanged
          values[f.key] = v;
        }
      }
      const res = await api(`/api/skills/${skill.name}/config`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ values }),
      });
      const body = await res.json();
      if (!res.ok) { msg.textContent = body.error; msg.className = 'error'; return; }
      msg.textContent = body.reload_error ? t('reload_failed') + body.reload_error : t('saved');
      msg.className = body.reload_error ? 'error' : 'notice';
      // The save reloaded the skill server-side — hints reflect the new
      // SAVED config now, so drop the stale marker and refetch.
      if (jd) refreshPhrases();
    },
  }), msg);
  app.replaceChildren(card);
}

async function uploadCard() {
  const card = el('section', { class: 'card' },
    el('h2', { text: t('upload_title') }),
    el('p', { class: 'help', text: t('upload_help') }),
  );
  const msg = el('p', { class: 'error' });
  const file = el('input', { type: 'file', accept: '.wasm' });
  card.append(file, el('button', {
    text: t('install'),
    onclick: async () => {
      if (!file.files.length) return;
      const form = new FormData();
      form.append('file', file.files[0]);
      const res = await api('/api/skills/upload', { method: 'POST', body: form });
      const body = await res.json();
      msg.textContent = res.ok
        ? (body.reload_error ? t('reload_failed') + body.reload_error : t('saved'))
        : body.error;
      msg.className = res.ok && !body.reload_error ? 'notice' : 'error';
      if (res.ok) renderList();
    },
  }));
  try {
    const bundled = await (await api('/api/bundled')).json();
    for (const b of bundled) {
      card.append(el('div', { class: 'skill-row' },
        el('span', { class: 'name', text: b.name }),
        el('button', {
          text: t('install'),
          onclick: async () => { await api(`/api/bundled/${b.name}/install`, { method: 'POST' }); renderList(); },
        }),
      ));
    }
  } catch {}
  card.append(msg);
  return card;
}

// Accent-insensitive haystack for the discovery search box — "temperature"
// must find "Température".
function searchable(s) {
  return String(s).toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '');
}

function renderDiscoveryTree(container, rooms, sensorsTable, actionsTable, shuttersTable) {
  const existing = new Set((sensorsTable?.getRows() || []).map((r) => Number(r.id)));
  const existingActions = new Set((actionsTable?.getRows() || []).map((r) => Number(r.on_id)));
  const existingShutters = new Set((shuttersTable?.getRows() || []).map((r) => Number(r.up_id)));
  const boxes = [];
  const actionBoxes = [];
  const shutterBoxes = [];
  container.replaceChildren();
  if (!rooms.length) {
    container.append(el('p', { class: 'help', text: t('nothing_discovered') }));
    return;
  }
  // Every result row is built once, then the toolbar filters by toggling a
  // .hidden class — so checked boxes survive any filter change (and a device
  // checked then filtered out is still added by "Add selection").
  const entries = []; // {row, roomSection, roomName, type, hay, box}
  const roomSections = [];
  const addEntry = (roomSection, roomName, type, box, row, hayParts) => {
    entries.push({ row, roomSection, roomName, type, box, hay: searchable(hayParts.join(' ')) });
    roomSection.list.append(row);
  };
  for (const room of rooms) {
    const roomName = room.name || '—';
    const count = el('span', { class: 'badge' });
    const list = el('div');
    const node = el('div', { class: 'disc-room' },
      el('div', { class: 'disc-room-head' }, el('span', { text: roomName }), count),
      list);
    const roomSection = { node, list, count, name: roomName };
    roomSections.push(roomSection);
    for (const eq of room.equipments) {
      for (const cmd of eq.cmds) {
        const box = el('input', { type: 'checkbox' });
        box.checked = existing.has(cmd.id);
        box.disabled = existing.has(cmd.id); // already mapped — keep it in the table
        boxes.push({ box, cmd, eqName: eq.name, room: room.name });
        const badge = cmd.subtype === 'binary' ? 'on/off' : (cmd.unit || '');
        addEntry(roomSection, roomName, 'sensor', box, el('label', { class: 'disc-row' },
          box,
          el('span', { class: 'name', text: `${eq.name} — ${cmd.name}` }),
          badge ? el('span', { class: 'badge', text: badge }) : '',
          el('span', { class: 'badge type-sensor', text: t('type_sensor') }),
        ), [eq.name, cmd.name, roomName]);
      }
      for (const act of (eq.actions || [])) {
        const box = el('input', { type: 'checkbox' });
        box.checked = existingActions.has(act.on_id);
        box.disabled = existingActions.has(act.on_id);
        actionBoxes.push({ box, act, eqName: eq.name, room: room.name });
        addEntry(roomSection, roomName, 'onoff', box, el('label', { class: 'disc-row' },
          box,
          el('span', { class: 'name', text: `${eq.name} — on/off` }),
          el('span', { class: 'badge type-onoff', text: t('action_onoff') }),
        ), [eq.name, roomName]);
      }
      for (const sh of (eq.shutters || [])) {
        const box = el('input', { type: 'checkbox' });
        box.checked = existingShutters.has(sh.up_id);
        box.disabled = existingShutters.has(sh.up_id);
        shutterBoxes.push({ box, sh, eqName: eq.name, room: room.name });
        const extras = [
          sh.stop_id !== undefined ? t('with_stop') : '',
          sh.slider_id !== undefined ? t('with_position') : '',
        ].filter(Boolean).join(' ');
        addEntry(roomSection, roomName, 'shutter', box, el('label', { class: 'disc-row' },
          box,
          el('span', { class: 'name', text: `${eq.name}${extras ? ` (${extras})` : ''}` }),
          el('span', { class: 'badge type-shutter', text: t('action_shutter') }),
        ), [eq.name, roomName]);
      }
    }
  }
  // --- Toolbar: search + room + type, with a live shown/total counter. ---
  const search = el('input', { type: 'search', class: 'disc-search', placeholder: t('filter_search'), autocomplete: 'off' });
  const roomSel = el('select', {},
    el('option', { value: '', text: t('filter_all_rooms') }),
    ...roomSections.map((s) => el('option', { value: s.name, text: s.name })));
  const typeSel = el('select', {},
    el('option', { value: '', text: t('filter_all_types') }),
    el('option', { value: 'sensor', text: t('type_sensor') }),
    el('option', { value: 'onoff', text: t('action_onoff') }),
    el('option', { value: 'shutter', text: t('action_shutter') }));
  const counter = el('span', { class: 'disc-count' });
  const empty = el('p', { class: 'help hidden', text: t('no_filter_match') });
  const addBtn = el('button', { text: t('add_selection') });
  const roomsWrap = el('div', { class: 'disc-rooms' }, ...roomSections.map((s) => s.node));
  const applyFilters = () => {
    const q = searchable(search.value.trim());
    const shownPerRoom = new Map();
    let shown = 0;
    for (const e of entries) {
      const visible = (!q || e.hay.includes(q))
        && (!roomSel.value || e.roomName === roomSel.value)
        && (!typeSel.value || e.type === typeSel.value);
      e.row.classList.toggle('hidden', !visible);
      if (visible) {
        shown += 1;
        shownPerRoom.set(e.roomSection, (shownPerRoom.get(e.roomSection) || 0) + 1);
      }
    }
    for (const s of roomSections) {
      const n = shownPerRoom.get(s) || 0;
      s.count.textContent = String(n);
      s.node.classList.toggle('hidden', n === 0);
    }
    counter.textContent = t('devices_shown')(shown, entries.length);
    empty.classList.toggle('hidden', shown > 0);
    roomsWrap.classList.toggle('hidden', shown === 0);
  };
  const refreshAddBtn = () => {
    const n = [...boxes, ...actionBoxes, ...shutterBoxes]
      .filter(({ box }) => box.checked && !box.disabled).length;
    addBtn.textContent = n ? `${t('add_selection')} (${n})` : t('add_selection');
    addBtn.disabled = n === 0;
  };
  search.addEventListener('input', applyFilters);
  roomSel.addEventListener('change', applyFilters);
  typeSel.addEventListener('change', applyFilters);
  roomsWrap.addEventListener('change', (e) => {
    if (e.target.type === 'checkbox') refreshAddBtn();
  });
  container.append(
    el('div', { class: 'disc-toolbar' }, search, roomSel, typeSel, counter),
    empty,
    roomsWrap,
    addBtn,
  );
  applyFilters();
  refreshAddBtn();
  addBtn.addEventListener('click', () => {
    const picked = boxes.filter(({ box }) => box.checked && !box.disabled);
    sensorsTable?.addRows(picked.map(({ cmd, eqName, room }) => ({
      name: composeSensorName(cmd.name, eqName, room),
      id: cmd.id,
      unit: cmd.unit || '',
      room: (room || '').toLowerCase(),
      prefix: guessRoomPrefix(room),
      kind: cmd.subtype === 'binary' ? 'binary' : 'numeric',
      on_label: cmd.on_label || '',
      off_label: cmd.off_label || '',
    })));
    const pickedActions = actionBoxes.filter(({ box }) => box.checked && !box.disabled);
    // 'état' is a generic command name, so the composed spoken name comes
    // from the EQUIPMENT name — "portail du garage" — like generic sensors.
    actionsTable?.addRows(pickedActions.map(({ act, eqName, room }) => ({
      name: composeSensorName('état', eqName, room),
      on_id: act.on_id,
      off_id: act.off_id,
      room: (room || '').toLowerCase(),
      prefix: guessRoomPrefix(room),
      confirm: false,
    })));
    const pickedShutters = shutterBoxes.filter(({ box }) => box.checked && !box.disabled);
    // Compose from the EQUIPMENT name (like generic sensors): equipment
    // "Volet" in room "Chambre" → "volet de la chambre". Absent stop/slider
    // ids stay undefined and are dropped by JSON.stringify on save.
    shuttersTable?.addRows(pickedShutters.map(({ sh, eqName, room }) => ({
      name: composeSensorName('état', eqName, room),
      up_id: sh.up_id,
      down_id: sh.down_id,
      stop_id: sh.stop_id,
      slider_id: sh.slider_id,
      room: (room || '').toLowerCase(),
      prefix: guessRoomPrefix(room),
      confirm: false,
    })));
    container.replaceChildren();
  });
}

renderList();
