function highlight(root, lane) {
  root.querySelectorAll("[data-graph-lane], [data-graph-row]").forEach((element) => {
    element.classList.toggle("graph-highlight", (element.dataset.graphLane ?? element.dataset.graphRow) === lane);
  });
  root.querySelectorAll("[data-graph-track]").forEach((track) => {
    track.setAttribute("aria-pressed", String(track.dataset.graphTrack === lane));
  });
}

// Track identities survive lane compaction, so highlighting follows a line even
// when branches merge or move horizontally. Delegation supports boosted pages.
document.addEventListener("click", (event) => {
  if (!(event.target instanceof Element)) return;
  const control = event.target.closest("[data-graph-track], [data-graph-clear], [data-graph-lane], [data-graph-hit]");
  const graph = control?.closest("[data-commit-graph]");
  if (!graph) return;
  const requested = control.dataset.graphTrack ?? control.dataset.graphLane ?? control.dataset.graphHit;
  const lane = requested === graph.dataset.graphSelected ? undefined : requested;
  if (lane === undefined) delete graph.dataset.graphSelected;
  else graph.dataset.graphSelected = lane;
  highlight(graph, lane);
  graph.querySelector("[data-graph-clear]").hidden = lane === undefined;
  graph.querySelector("[data-graph-status]").textContent = lane === undefined
    ? "Follow colored lines to trace history. Select a track to highlight its commits."
    : `Highlighting track ${Number(lane) + 1}. Select it again to show all history.`;
});

const pending = new WeakSet();
const observer = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver((entries) => {
  for (const entry of entries) {
    if (entry.isIntersecting) loadMore(entry.target);
  }
}, { rootMargin: "0px 0px 400px" });

function observeNext(root = document) {
  observer?.disconnect();
  root.querySelectorAll("[data-graph-more]:not([hidden], [data-graph-retry])").forEach((link) => observer?.observe(link));
}

async function loadMore(link) {
  if (!link.isConnected || pending.has(link) || link.hidden) return;
  const graph = link.closest("[data-commit-graph]");
  const footer = link.closest("[data-graph-pagination]");
  const error = footer.querySelector("[data-graph-error]");
  pending.add(link);
  observer?.unobserve(link);
  footer.setAttribute("aria-busy", "true");
  error.hidden = true;
  try {
    const response = await fetch(link.href);
    if (!response.ok) throw new Error("Commit graph request failed");
    const page = new DOMParser().parseFromString(await response.text(), "text/html");
    if (!link.isConnected) return;
    const next = page.querySelector("[data-commit-graph]");
    if (next?.dataset.graphSnapshot !== graph.dataset.graphSnapshot) {
      const reload = footer.querySelector("[data-graph-reload]");
      reload.hidden = false;
      if (document.activeElement === link) reload.focus({ preventScroll: true });
      link.hidden = true;
      error.hidden = false;
      return;
    }
    const currentRows = graph.querySelector("tbody");
    const newRows = next.querySelector("tbody");
    highlight(newRows, graph.dataset.graphSelected);
    const rows = Array.from(newRows.rows);
    const focus = document.activeElement === link && rows[0]?.querySelector(".commit-graph-summary");
    currentRows.append(...rows);
    for (const row of rows) window.htmx?.process(row);
    // All rows use the same scale so tracks line up when an older merge widens the graph.
    const width = next.dataset.graphWidth;
    if (width !== graph.dataset.graphWidth) {
      graph.dataset.graphWidth = width;
      graph.querySelector(".commit-graph-column").classList.toggle("commit-graph-column-compact", Number(width) <= 112);
      for (const svg of currentRows.querySelectorAll(".commit-graph-lanes svg")) {
        svg.setAttribute("width", width);
        svg.setAttribute("viewBox", `0 0 ${width} 72`);
      }
    }
    const nextFooter = next.querySelector("[data-graph-pagination]");
    if (nextFooter) {
      footer.replaceWith(nextFooter);
      window.htmx?.process(nextFooter);
      observeNext(graph);
    } else {
      footer.remove();
    }
    if (focus) focus.focus({ preventScroll: true });
  } catch {
    // Leave the link available for a manual retry, without repeatedly retrying on scroll.
    link.dataset.graphRetry = "";
    error.hidden = false;
  } finally {
    footer.removeAttribute("aria-busy");
    pending.delete(link);
  }
}

document.addEventListener("click", (event) => {
  if (!(event.target instanceof Element) || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  const link = event.target.closest("[data-graph-more]");
  if (!link) return;
  event.preventDefault();
  loadMore(link);
});

observeNext();
document.addEventListener("htmx:afterSettle", () => observeNext());
