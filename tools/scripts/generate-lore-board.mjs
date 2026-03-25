#!/usr/bin/env node

/**
 * Generates lore/BOARD.md (visual backlog board) and lore/board.json (data for HTML board).
 *
 * Usage: node tools/scripts/generate-lore-board.mjs
 *
 * Reads all tasks from lore/1-tasks/{backlog,active,blocked,archive}
 * and produces a rich Markdown board + JSON index.
 */

import { readdirSync, readFileSync, writeFileSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, '..', '..');
const TASKS_DIR = join(ROOT, 'lore', '1-tasks');
const OUT_MD = join(ROOT, 'lore', 'BOARD.md');
const OUT_JSON = join(ROOT, 'lore', 'board.json');

const STATUS_DIRS = ['backlog', 'active', 'blocked', 'archive'];

const LAYER_LABELS = {
  'layer-research': 'Research',
  'layer-domain': 'Domain',
  'layer-database': 'Database',
  'layer-backend': 'Backend API',
  'layer-indexing': 'Indexing',
  'layer-frontend': 'Frontend',
  'layer-infra': 'Infrastructure',
};

const LAYER_ORDER = Object.keys(LAYER_LABELS);

const LAYER_EMOJI = {
  'layer-research': '🔬',
  'layer-domain': '📦',
  'layer-database': '🗄️',
  'layer-backend': '⚙️',
  'layer-indexing': '🔄',
  'layer-frontend': '🖥️',
  'layer-infra': '☁️',
};

const STATUS_EMOJI = {
  backlog: '📋',
  active: '🚧',
  blocked: '🚫',
  completed: '✅',
  canceled: '❌',
  superseded: '🔀',
};

function parseFrontmatter(content) {
  const match = content.match(/^---\n([\s\S]*?)\n---/);
  if (!match) return null;

  const lines = match[1].split('\n');
  const meta = {};
  let currentKey = null;
  let currentList = null;

  for (const line of lines) {
    const kvMatch = line.match(/^(\w[\w_]*):\s*(.*)$/);
    if (kvMatch) {
      const [, key, rawValue] = kvMatch;
      const value = rawValue.trim().replace(/^['"](.*)['"]$/, '$1');

      if (value === '' || value === '[]') {
        meta[key] = value === '[]' ? [] : '';
        currentKey = key;
        currentList = value === '[]' ? [] : null;
      } else if (value.startsWith('[') && value.endsWith(']')) {
        meta[key] = value
          .slice(1, -1)
          .split(',')
          .map((s) => s.trim().replace(/^['"](.*)['"]$/, '$1'))
          .filter(Boolean);
        currentKey = key;
        currentList = null;
      } else {
        meta[key] = value;
        currentKey = key;
        currentList = null;
      }
    } else if (line.match(/^\s+-\s+(.*)$/)) {
      const item = line
        .match(/^\s+-\s+(.*)$/)[1]
        .trim()
        .replace(/^['"](.*)['"]$/, '$1');
      if (currentList === null) {
        currentList = [item];
        meta[currentKey] = currentList;
      } else {
        currentList.push(item);
      }
    }
  }
  return meta;
}

function loadTasks() {
  const tasks = [];

  for (const statusDir of STATUS_DIRS) {
    const dir = join(TASKS_DIR, statusDir);
    let entries;
    try {
      entries = readdirSync(dir);
    } catch {
      continue;
    }

    for (const entry of entries) {
      if (entry.startsWith('_') || entry === 'CLAUDE.md') continue;

      const fullPath = join(dir, entry);
      const stat = statSync(fullPath);
      let content;

      if (stat.isDirectory()) {
        const readmePath = join(fullPath, 'README.md');
        try {
          content = readFileSync(readmePath, 'utf-8');
        } catch {
          continue;
        }
      } else if (entry.endsWith('.md')) {
        content = readFileSync(fullPath, 'utf-8');
      } else {
        continue;
      }

      const meta = parseFrontmatter(content);
      if (!meta || !meta.id) continue;

      meta._dir = statusDir;
      meta._path = stat.isDirectory()
        ? `lore/1-tasks/${statusDir}/${entry}/README.md`
        : `lore/1-tasks/${statusDir}/${entry}`;
      meta._relpath = stat.isDirectory()
        ? `1-tasks/${statusDir}/${entry}/README.md`
        : `1-tasks/${statusDir}/${entry}`;
      tasks.push(meta);
    }
  }

  tasks.sort((a, b) => {
    const na = parseInt(a.id, 10);
    const nb = parseInt(b.id, 10);
    return na - nb;
  });

  return tasks;
}

function getLayer(task) {
  const tags = Array.isArray(task.tags) ? task.tags : [];
  const layerTag = tags.find((t) => t.startsWith('layer-')) || 'layer-other';
  // Normalize sub-layers (e.g. layer-frontend-pages → layer-frontend)
  for (const key of LAYER_ORDER) {
    if (layerTag === key || layerTag.startsWith(key + '-')) return key;
  }
  return layerTag;
}

function getPriority(task) {
  const tags = Array.isArray(task.tags) ? task.tags : [];
  const p = tags.find((t) => t.startsWith('priority-'));
  return p ? p.replace('priority-', '') : 'medium';
}

function getAssignee(task) {
  const history = Array.isArray(task.history) ? task.history : [];
  const lastEntry = history[history.length - 1];
  return lastEntry?.who || null;
}

function generateMarkdown(tasks) {
  const now = new Date().toISOString().slice(0, 10);

  const byStatus = {};
  for (const t of tasks) {
    const s = t._dir;
    if (!byStatus[s]) byStatus[s] = [];
    byStatus[s].push(t);
  }

  const byLayer = {};
  for (const t of tasks) {
    const l = getLayer(t);
    if (!byLayer[l]) byLayer[l] = [];
    byLayer[l].push(t);
  }

  const total = tasks.length;
  const backlogCount = (byStatus.backlog || []).length;
  const activeCount = (byStatus.active || []).length;
  const blockedCount = (byStatus.blocked || []).length;
  const doneCount = (byStatus.archive || []).length;

  let md = '';

  // Header
  md += `# Backlog Board\n\n`;
  md += `> **Auto-generated** — do not edit manually.\n`;
  md += `> Run \`node tools/scripts/generate-lore-board.mjs\` to regenerate.\n`;
  md += `> Last updated: ${now}\n\n`;

  // Stats
  md += `## Overview\n\n`;
  md += `| Total | ${STATUS_EMOJI.backlog} Backlog | ${STATUS_EMOJI.active} Active | ${STATUS_EMOJI.blocked} Blocked | ${STATUS_EMOJI.completed} Done |\n`;
  md += `| :---: | :---: | :---: | :---: | :---: |\n`;
  md += `| **${total}** | ${backlogCount} | ${activeCount} | ${blockedCount} | ${doneCount} |\n\n`;

  // Progress bar
  const pctDone = total > 0 ? Math.round((doneCount / total) * 100) : 0;
  const pctActive = total > 0 ? Math.round((activeCount / total) * 100) : 0;
  const pctBlocked = total > 0 ? Math.round((blockedCount / total) * 100) : 0;
  md += `**Progress:** ${pctDone}% complete`;
  if (activeCount > 0) md += ` · ${pctActive}% in progress`;
  if (blockedCount > 0) md += ` · ${pctBlocked}% blocked`;
  md += `\n\n`;

  // Layer breakdown
  md += `## By Layer\n\n`;
  md += `| Layer | Total | Backlog | Active | Blocked | Done |\n`;
  md += `| :--- | :---: | :---: | :---: | :---: | :---: |\n`;

  for (const layerKey of LAYER_ORDER) {
    const layerTasks = byLayer[layerKey] || [];
    if (layerTasks.length === 0) continue;
    const emoji = LAYER_EMOJI[layerKey] || '';
    const label = LAYER_LABELS[layerKey] || layerKey;
    const lb = layerTasks.filter((t) => t._dir === 'backlog').length;
    const la = layerTasks.filter((t) => t._dir === 'active').length;
    const lbl = layerTasks.filter((t) => t._dir === 'blocked').length;
    const ld = layerTasks.filter((t) => t._dir === 'archive').length;
    md += `| ${emoji} ${label} | ${layerTasks.length} | ${lb} | ${la} | ${lbl} | ${ld} |\n`;
  }
  md += `\n`;

  // Active tasks (kanban highlight)
  if (activeCount > 0) {
    md += `## 🚧 Active\n\n`;
    md += `| ID | Title | Layer | Assignee | Since |\n`;
    md += `| :--- | :--- | :--- | :--- | :--- |\n`;
    for (const t of byStatus.active || []) {
      const layer = LAYER_LABELS[getLayer(t)] || '';
      const assignee = getAssignee(t) || '—';
      const history = Array.isArray(t.history) ? t.history : [];
      const since = history[history.length - 1]?.date || '—';
      md += `| [${t.id}](${t._relpath}) | ${t.title} | ${layer} | \`${assignee}\` | ${since} |\n`;
    }
    md += `\n`;
  }

  // Blocked tasks
  if (blockedCount > 0) {
    md += `## 🚫 Blocked\n\n`;
    md += `| ID | Title | Blocked By |\n`;
    md += `| :--- | :--- | :--- |\n`;
    for (const t of byStatus.blocked || []) {
      const history = Array.isArray(t.history) ? t.history : [];
      const lastEntry = history[history.length - 1];
      const by = lastEntry?.by
        ? lastEntry.by.map((b) => `\`${b}\``).join(', ')
        : lastEntry?.note || '—';
      md += `| [${t.id}](${t._relpath}) | ${t.title} | ${by} |\n`;
    }
    md += `\n`;
  }

  // Full backlog by layer
  md += `## 📋 Backlog\n\n`;

  for (const layerKey of LAYER_ORDER) {
    const layerTasks = (byLayer[layerKey] || []).filter(
      (t) => t._dir === 'backlog'
    );
    if (layerTasks.length === 0) continue;

    const emoji = LAYER_EMOJI[layerKey] || '';
    const label = LAYER_LABELS[layerKey] || layerKey;
    md += `### ${emoji} ${label}\n\n`;
    md += `| ID | Title | Priority | Type | Depends On |\n`;
    md += `| :--- | :--- | :---: | :---: | :--- |\n`;

    for (const t of layerTasks) {
      const priority = getPriority(t);
      const priorityBadge =
        priority === 'high' ? '🔴' : priority === 'low' ? '⚪' : '🟡';
      const deps = Array.isArray(t.related_tasks)
        ? t.related_tasks.map((d) => `\`${d}\``).join(', ')
        : '—';
      md += `| [${t.id}](${t._relpath}) | ${
        t.title
      } | ${priorityBadge} ${priority} | ${t.type} | ${deps || '—'} |\n`;
    }
    md += `\n`;
  }

  // Done
  if (doneCount > 0) {
    md += `## ✅ Completed\n\n`;
    md += `| ID | Title | Completed |\n`;
    md += `| :--- | :--- | :--- |\n`;
    for (const t of byStatus.archive || []) {
      const history = Array.isArray(t.history) ? t.history : [];
      const lastEntry = history[history.length - 1];
      md += `| [${t.id}](${t._relpath}) | ${t.title} | ${
        lastEntry?.date || '—'
      } |\n`;
    }
    md += `\n`;
  }

  // Dependency graph (Mermaid)
  md += `## Dependency Graph\n\n`;
  md += '```mermaid\ngraph LR\n';

  // Style definitions
  md += `  classDef research fill:#e1f5fe,stroke:#0288d1\n`;
  md += `  classDef domain fill:#f3e5f5,stroke:#7b1fa2\n`;
  md += `  classDef database fill:#fff3e0,stroke:#f57c00\n`;
  md += `  classDef backend fill:#e8f5e9,stroke:#388e3c\n`;
  md += `  classDef indexing fill:#fce4ec,stroke:#c62828\n`;
  md += `  classDef frontend fill:#e0f2f1,stroke:#00695c\n`;
  md += `  classDef infra fill:#efebe9,stroke:#4e342e\n`;

  const layerToClass = {
    'layer-research': 'research',
    'layer-domain': 'domain',
    'layer-database': 'database',
    'layer-backend': 'backend',
    'layer-indexing': 'indexing',
    'layer-frontend': 'frontend',
    'layer-infra': 'infra',
  };

  // Only show tasks that have dependencies or are depended upon
  const hasDeps = new Set();
  const isDep = new Set();
  for (const t of tasks) {
    const deps = Array.isArray(t.related_tasks) ? t.related_tasks : [];
    if (deps.length > 0) {
      hasDeps.add(t.id);
      for (const d of deps) isDep.add(d);
    }
  }
  const graphTasks = tasks.filter((t) => hasDeps.has(t.id) || isDep.has(t.id));

  for (const t of graphTasks) {
    const shortTitle =
      t.title.length > 35 ? t.title.slice(0, 32) + '...' : t.title;
    const cls = layerToClass[getLayer(t)] || '';
    md += `  T${t.id}["${t.id}: ${shortTitle.replace(/"/g, "'")}"]\n`;
    if (cls) md += `  class T${t.id} ${cls}\n`;
  }

  for (const t of tasks) {
    const deps = Array.isArray(t.related_tasks) ? t.related_tasks : [];
    for (const dep of deps) {
      if (graphTasks.some((g) => g.id === dep)) {
        md += `  T${dep} --> T${t.id}\n`;
      }
    }
  }

  md += '```\n\n';

  // Legend
  md += `**Legend:** `;
  md += Object.entries(LAYER_EMOJI)
    .map(([k, e]) => `${e} ${LAYER_LABELS[k]}`)
    .join(' · ');
  md += ` | 🔴 High · 🟡 Medium · ⚪ Low\n`;

  return md;
}

function generateJSON(tasks) {
  return tasks.map((t) => ({
    id: t.id,
    title: t.title,
    type: t.type,
    status: t._dir,
    layer: getLayer(t),
    priority: getPriority(t),
    assignee: getAssignee(t),
    tags: t.tags || [],
    related_tasks: t.related_tasks || [],
    path: t._path,
    history: t.history || [],
  }));
}

// Main
const tasks = loadTasks();
const md = generateMarkdown(tasks);
const json = generateJSON(tasks);

writeFileSync(OUT_MD, md);
writeFileSync(
  OUT_JSON,
  JSON.stringify({ generated: new Date().toISOString(), tasks: json }, null, 2)
);

console.log(`BOARD.md generated (${tasks.length} tasks)`);
console.log(`board.json generated`);
