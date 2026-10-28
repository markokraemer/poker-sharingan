// Shared config. ARENA_API can be overridden with ?api=URL (for local testing).
const ARENA_API = (function () {
  try { const q = new URLSearchParams(location.search).get("api"); if (q) return q.replace(/\/+$/, ""); } catch (e) {}
  return "https://arena.sharingan.poker";
})();
const RULE = "=".repeat(120);
function esc(s) { return String(s).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])); }
// Keep ?api= when moving between pages.
function keepApi() {
  const q = new URLSearchParams(location.search).get("api");
  if (!q) return;
  document.querySelectorAll("a[data-int]").forEach(a => { a.href = a.getAttribute("href").split("?")[0] + "?api=" + encodeURIComponent(q); });
}
document.addEventListener("DOMContentLoaded", keepApi);
