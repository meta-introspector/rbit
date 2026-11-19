import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../../crates/rbit-web/assets/branch-graph.js', import.meta.url), 'utf8');
function mount() {
  const listeners = new Map();
  class Element {
    constructor(dataset = {}) { this.dataset = dataset; this.attributes = {}; this.classes = new Set(); this.classList = { toggle: (name, active) => active ? this.classes.add(name) : this.classes.delete(name) }; }
    closest(selector) {
      if (selector === '[data-commit-graph]') return graph;
      for (const attr of selector.split(', ')) {
        const key = attr.slice(6, -1).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
        if (key in this.dataset) return this;
      }
      return null;
    }
    setAttribute(key, value) { this.attributes[key] = value; }
  }
  const tracks = [new Element({ graphTrack: '0' }), new Element({ graphTrack: '1' }), new Element({ graphTrack: '0' })];
  const lines = [new Element({ graphLane: '0' }), new Element({ graphLane: '1' }), new Element({ graphRow: '0' })];
  const clear = new Element({ graphClear: '' });
  clear.hidden = true;
  const status = new Element();
  const graph = {
    dataset: {},
    querySelectorAll: (selector) => selector === '[data-graph-track]' ? tracks : lines,
    querySelector: (selector) => selector === '[data-graph-clear]' ? clear : status,
  };
  const document = {
    querySelectorAll: () => [],
    addEventListener(type, handler) {
      const handlers = listeners.get(type) ?? [];
      handlers.push(handler);
      listeners.set(type, handlers);
    },
  };
  vm.runInNewContext(source, { document, Element });
  const click = (target) => {
    for (const handler of listeners.get('click')) handler({ target, button: 0 });
  };
  return { click, tracks, lines, clear, status, graph, hit: new Element({ graphHit: '1' }), blank: new Element() };
}

test('selecting a track highlights its connected edges and commits after lane shifts', () => {
  const page = mount();
  page.click(page.tracks[0]);
  assert.equal(page.graph.dataset.graphSelected, '0');
  assert.deepEqual(page.lines.map((line) => line.classes.has('graph-highlight')), [true, false, true]);
  assert.deepEqual(page.tracks.map((track) => track.attributes['aria-pressed']), ['true', 'false', 'true']);
  assert.match(page.status.textContent, /Highlighting track 1/);
  assert.equal(page.clear.hidden, false);
  page.click(page.tracks[1]);
  assert.deepEqual(page.lines.map((line) => line.classes.has('graph-highlight')), [false, true, false]);
});

test('repeat selection and clear reset highlighting and accessible state', () => {
  const page = mount();
  page.click(page.tracks[0]);
  page.click(page.tracks[0]);
  assert.equal(page.graph.dataset.graphSelected, undefined);
  assert.ok(page.lines.every((line) => !line.classes.has('graph-highlight')));
  assert.equal(page.clear.hidden, true);
  page.click(page.tracks[1]);
  page.click(page.clear);
  assert.equal(page.graph.dataset.graphSelected, undefined);
  assert.ok(page.tracks.every((track) => track.attributes['aria-pressed'] === 'false'));
  assert.match(page.status.textContent, /Select a track/);
});

test('clicking a graph path or its wider hit area toggles the same track as its button', () => {
  const page = mount();
  page.click(page.lines[1]);
  assert.equal(page.graph.dataset.graphSelected, '1');
  assert.deepEqual(page.tracks.map((track) => track.attributes['aria-pressed']), ['false', 'true', 'false']);
  page.click(page.blank);
  page.click(page.lines[2]);
  assert.equal(page.graph.dataset.graphSelected, '1', 'blank space and commit rows do not toggle a track');
  page.click(page.hit);
  assert.equal(page.graph.dataset.graphSelected, undefined);
  page.click(page.hit);
  assert.ok(page.lines[1].classes.has('graph-highlight'));
  page.click(page.tracks[1]);
  assert.equal(page.graph.dataset.graphSelected, undefined);
});
