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
  },
};
const lang = (navigator.language || 'en').startsWith('fr') ? 'fr' : 'en';
const t = (k) => T[lang][k] || k;

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

// Normalized phrases that appear under MORE THAN ONE per-sensor intent
// (jeedom.read.{id}) — name collisions like six sensors all called
// "température". groups: intent -> {locale -> [phrases]}.
function duplicatePhrases(groups) {
  const firstIntent = new Map();
  const dupes = new Set();
  for (const [intent, locales] of Object.entries(groups)) {
    if (!/^jeedom\.read\.\d+$/.test(intent)) continue;
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
        ...f.item_fields.map((c) => el('th', { text: c.key })),
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
  const widgets = fields.map((f) => {
    const w = fieldInput(f, skill.config[f.key], f.key === 'sensors' ? sensorOpts : undefined);
    card.append(w);
    return [f, w];
  });
  if (skill.name === 'jeedom') {
    const jmsg = el('p', { class: 'help' });
    const tree = el('div');
    card.append(
      el('button', {
        class: 'quiet', text: t('test_connection'),
        onclick: async () => {
          jmsg.textContent = t('testing');
          const body = await (await api('/api/skills/jeedom/test', { method: 'POST' })).json();
          jmsg.textContent = body.status === 'ok'
            ? t('jeedom_ok')
            : t(`jeedom_${body.status}`);
        },
      }),
      el('button', {
        class: 'quiet', text: t('discover'),
        onclick: async () => {
          jmsg.textContent = t('discovering');
          const body = await (await api('/api/skills/jeedom/discover', { method: 'POST' })).json();
          if (body.status !== 'ok') { jmsg.textContent = t(`jeedom_${body.status}`); return; }
          jmsg.textContent = '';
          renderDiscoveryTree(tree, body.rooms, findSensorsTable());
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
          renderDiscoveryTree(tree, body.rooms, table);
          table?.rerender();
        },
      }),
      jmsg, pmsg, tree,
    );
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

function renderDiscoveryTree(container, rooms, sensorsTable) {
  const existing = new Set((sensorsTable?.getRows() || []).map((r) => Number(r.id)));
  const boxes = [];
  container.replaceChildren();
  if (!rooms.length) {
    container.append(el('p', { class: 'help', text: t('nothing_discovered') }));
    return;
  }
  for (const room of rooms) {
    const section = el('div', {}, el('label', { text: room.name || '—' }));
    for (const eq of room.equipments) {
      for (const cmd of eq.cmds) {
        const box = el('input', { type: 'checkbox' });
        box.checked = existing.has(cmd.id);
        box.disabled = existing.has(cmd.id); // already mapped — keep it in the table
        boxes.push({ box, cmd, eqName: eq.name, room: room.name });
        const badge = cmd.subtype === 'binary' ? 'on/off' : (cmd.unit || '');
        section.append(el('div', { class: 'skill-row' },
          box,
          el('span', { class: 'name', text: `${eq.name} — ${cmd.name}` }),
          badge ? el('span', { class: 'badge', text: badge }) : '',
        ));
      }
    }
    container.append(section);
  }
  container.append(el('button', {
    text: t('add_selection'),
    onclick: () => {
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
      container.replaceChildren();
    },
  }));
}

renderList();
