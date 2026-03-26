#!/usr/bin/env node

/**
 * Generates lore/board.json (data for HTML board deployed to GitHub Pages).
 *
 * Usage: node tools/scripts/generate-lore-board.mjs
 *
 * Reads all tasks from lore/1-tasks/{backlog,active,blocked,archive}
 * and produces a JSON index consumed by board.html.
 */

import { readdirSync, readFileSync, writeFileSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, '..', '..');
const TASKS_DIR = join(ROOT, 'lore', '1-tasks');
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
  'layer-tooling': 'Tooling',
};

const LAYER_ORDER = Object.keys(LAYER_LABELS);

function parseFrontmatter(content) {
  const match = content.match(/^---\n([\s\S]*?)\n---/);
  if (!match) return null;

  const lines = match[1].split('\n');
  const meta = {};
  let currentKey = null;
  let currentList = null;
  let currentObj = null;

  for (const line of lines) {
    // Top-level key: value
    const kvMatch = line.match(/^(\w[\w_]*):\s*(.*)$/);
    if (kvMatch) {
      currentObj = null;
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
      continue;
    }

    // List item starting with "- key: value" (object in list)
    const listObjMatch = line.match(/^\s+-\s+(\w[\w_]*):\s*(.*)$/);
    if (listObjMatch) {
      const [, k, rawV] = listObjMatch;
      const v = rawV.trim().replace(/^['"](.*)['"]$/, '$1');
      currentObj = { [k]: v };
      if (currentList === null) {
        currentList = [currentObj];
        meta[currentKey] = currentList;
      } else {
        currentList.push(currentObj);
      }
      continue;
    }

    // Continuation key inside list object: "    key: value"
    const contMatch = line.match(/^\s{4,}(\w[\w_]*):\s*(.*)$/);
    if (contMatch && currentObj) {
      const [, k, rawV] = contMatch;
      currentObj[k] = rawV.trim().replace(/^['"](.*)['"]$/, '$1');
      continue;
    }

    // Simple list item: "  - value"
    const simpleListMatch = line.match(/^\s+-\s+(.*)$/);
    if (simpleListMatch) {
      const item = simpleListMatch[1].trim().replace(/^['"](.*)['"]$/, '$1');
      currentObj = null;
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

      meta._description = extractDescription(content);
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
  if (task._dir !== 'active' && task._dir !== 'archive') return null;
  const history = Array.isArray(task.history) ? task.history : [];
  const lastEntry = history[history.length - 1];
  return lastEntry?.who || null;
}

function extractDescription(content) {
  // Extract text between "## Summary" and the next "##" heading
  const match = content.match(/## Summary\n\n([\s\S]*?)(?=\n## |\n---$)/);
  if (match) return match[1].trim();
  // Fallback: first paragraph after frontmatter
  const body = content
    .replace(/^---[\s\S]*?---\n*/, '')
    .replace(/^#[^\n]*\n*/, '');
  const para = body.match(/^([^\n#][\s\S]*?)(?=\n\n|\n#|$)/);
  return para ? para[1].trim() : '';
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
    related_adr: t.related_adr || [],
    path: t._path,
    history: t.history || [],
    description: t._description || '',
  }));
}

// Main
const tasks = loadTasks();
const json = generateJSON(tasks);

writeFileSync(
  OUT_JSON,
  JSON.stringify({ generated: new Date().toISOString(), tasks: json }, null, 2)
);

console.log(`board.json generated (${tasks.length} tasks)`);
