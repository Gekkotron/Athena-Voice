'use strict';

const T = {
  en: {
    token_title: 'Admin token', save: 'Save', skills: 'Skills',
    token_help: 'Paste the token printed in the terminal on first start.',
    bad_token: 'That token was not accepted.',
    enabled: 'enabled', disabled: 'disabled', loaded: 'loaded', not_loaded: 'not loaded',
    enable: 'Enable', disable: 'Disable', back: '← Back', add_row: 'Add row', remove: 'Remove',
    saved: 'Saved.', reload_failed: 'Saved, but reload failed: ',
    secret_set: 'A value is stored. Leave blank to keep it.',
    upload_title: 'Install a skill', upload_help: 'Drop a .wasm file or pick a bundled skill.',
    install: 'Install', no_settings: 'This skill has no settings.',
    needs_config: 'needs config', key: 'Key', value: 'Value',
    test_connection: 'Test connection', testing: 'Testing…',
    jeedom_ok: 'Jeedom reachable, version ', jeedom_unauthorized: 'Invalid API key',
    jeedom_unreachable: 'Jeedom unreachable — check the URL', jeedom_bad_response: 'Unexpected reply — is this a Jeedom URL?',
    jeedom_unconfigured: 'Save the URL and API key first',
    discover: 'Discover sensors', discovering: 'Scanning…',
    add_selection: 'Add selection', nothing_discovered: 'No readable commands found',
  },
  fr: {
    token_title: 'Jeton administrateur', save: 'Enregistrer', skills: 'Compétences',
    token_help: 'Collez le jeton affiché dans le terminal au premier démarrage.',
    bad_token: 'Jeton refusé.',
    enabled: 'activée', disabled: 'désactivée', loaded: 'chargée', not_loaded: 'non chargée',
    enable: 'Activer', disable: 'Désactiver', back: '← Retour', add_row: 'Ajouter', remove: 'Retirer',
    saved: 'Enregistré.', reload_failed: 'Enregistré, mais rechargement échoué : ',
    secret_set: 'Une valeur est enregistrée. Laissez vide pour la conserver.',
    upload_title: 'Installer une compétence', upload_help: 'Déposez un fichier .wasm ou choisissez une compétence fournie.',
    install: 'Installer', no_settings: 'Cette compétence n’a aucun réglage.',
    needs_config: 'à configurer', key: 'Clé', value: 'Valeur',
    test_connection: 'Tester la connexion', testing: 'Test en cours…',
    jeedom_ok: 'Jeedom joignable, version ', jeedom_unauthorized: 'Clé API invalide',
    jeedom_unreachable: 'Jeedom injoignable — vérifiez l’URL', jeedom_bad_response: 'Réponse inattendue — est-ce bien une URL Jeedom ?',
    jeedom_unconfigured: 'Enregistrez d’abord l’URL et la clé API',
    discover: 'Découvrir les capteurs', discovering: 'Analyse en cours…',
    add_selection: 'Ajouter la sélection', nothing_discovered: 'Aucune commande lisible trouvée',
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
function composeSensorName(cmdName, room) {
  const cmd = cmdName.toLowerCase();
  if (!room) return cmd;
  const r = room.toLowerCase();
  const article = /^[aeéèiouy]/.test(r) ? 'de l’' : FR_ROOM_ARTICLES[r];
  if (!article) return `${cmd} ${r}`;
  return article.endsWith('’') ? `${cmd} ${article}${r}` : `${cmd} ${article} ${r}`;
}

const app = document.getElementById('app');
let token = localStorage.getItem('athena-admin-token') || '';

async function api(path, opts = {}) {
  let res;
  try {
    res = await fetch(path, {
      ...opts,
      headers: { Authorization: `Bearer ${token}`, ...(opts.headers || {}) },
    });
  } catch {
    document.getElementById('status').textContent = 'API unreachable';
    throw new Error('network error');
  }
  if (res.status === 401) { renderTokenPrompt(true); throw new Error('unauthorized'); }
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

function renderTokenPrompt(failed = false) {
  app.replaceChildren(document.getElementById('tpl-token').content.cloneNode(true));
  app.querySelectorAll('[data-i18n]').forEach((n) => (n.textContent = t(n.dataset.i18n)));
  if (failed) document.getElementById('token-error').textContent = t('bad_token');
  document.getElementById('token-save').onclick = async () => {
    token = document.getElementById('token-input').value.trim();
    localStorage.setItem('athena-admin-token', token);
    renderList();
  };
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

function fieldInput(f, current) {
  if (f.type === 'list') return listEditor(f, current);
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

function listEditor(f, current) {
  let rows = [];
  try { rows = current && current.kind === 'plain' ? JSON.parse(current.value) : []; } catch {}
  const table = el('table', { 'data-list': f.key });
  const render = () => {
    table.replaceChildren(
      el('tr', {}, ...f.item_fields.map((c) => el('th', { text: c.key })), el('th')),
      ...rows.map((row, i) => el('tr', {},
        ...f.item_fields.map((c) => {
          const cell = el('input', { type: c.type === 'number' ? 'number' : 'text' });
          cell.value = row[c.key] ?? '';
          cell.oninput = () => { row[c.key] = c.type === 'number' ? Number(cell.value) : cell.value; };
          return el('td', {}, cell);
        }),
        el('td', {}, el('button', { class: 'quiet', text: t('remove'), onclick: () => { rows.splice(i, 1); render(); } })),
      )),
    );
  };
  render();
  table.getRows = () => rows;
  table.addRows = (newRows) => { rows.push(...newRows); render(); };
  return el('div', {},
    el('label', { text: f.label || f.key }), table,
    el('button', { class: 'quiet', text: t('add_row'), onclick: () => { rows.push({}); render(); } }),
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
  const widgets = fields.map((f) => { const w = fieldInput(f, skill.config[f.key]); card.append(w); return [f, w]; });
  if (skill.name === 'jeedom') {
    const jmsg = el('p', { class: 'help' });
    const tree = el('div');
    const findSensorsTable = () =>
      widgets.find(([f]) => f.key === 'sensors')?.[1].querySelector('table');
    card.append(
      el('button', {
        class: 'quiet', text: t('test_connection'),
        onclick: async () => {
          jmsg.textContent = t('testing');
          const body = await (await api('/api/skills/jeedom/test', { method: 'POST' })).json();
          jmsg.textContent = body.status === 'ok'
            ? t('jeedom_ok') + body.version
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
        boxes.push({ box, cmd, room: room.name });
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
      sensorsTable?.addRows(picked.map(({ cmd, room }) => ({
        name: composeSensorName(cmd.name, room),
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

if (token) renderList(); else renderTokenPrompt();
