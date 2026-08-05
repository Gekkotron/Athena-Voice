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
function composeSensorName(cmdName, eqName, room) {
  const isGeneric = GENERIC_CMD_NAMES.includes(cmdName.toLowerCase());
  const cmd = (isGeneric ? eqName : cmdName).toLowerCase();
  if (!room) return cmd;
  const r = room.toLowerCase();
  const article = /^[aeéèiouy]/.test(r) ? 'de l’' : FR_ROOM_ARTICLES[r];
  if (!article) return `${cmd} ${r}`;
  return article.endsWith('’') ? `${cmd} ${article}${r}` : `${cmd} ${article} ${r}`;
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
  app.replaceChildren(list, await uploadCard());
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
            cell.oninput = () => { row[c.key] = c.type === 'number' ? Number(cell.value) : cell.value; edited(); };
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
  } : undefined;
  function sensorDetail() { return null; } // replaced by the phrase-hints task
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
      jmsg, tree,
    );
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
        kind: cmd.subtype === 'binary' ? 'binary' : 'numeric',
        on_label: cmd.on_label || '',
        off_label: cmd.off_label || '',
      })));
      container.replaceChildren();
    },
  }));
}

renderList();
