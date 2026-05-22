/**
 * Resolve `assets/` for local HTML editors under `tools/`.
 * Probes `data/relics.json` so paths work when the page URL is e.g.
 * `http://localhost:8000/Mahjuro/tools/relic_flavor_editor.html`.
 */
(function (global) {
  /** @type {URL | null} */
  let resolvedAssetsBase = null;

  function candidateAssetsBases() {
    const href = global.location.href;
    const pathname = global.location.pathname;
    const out = [];
    const toolsMatch = pathname.match(/^(.*)\/tools\/[^/]+$/);
    if (toolsMatch && toolsMatch[1]) {
      out.push(new URL(`${toolsMatch[1]}/assets/`, href));
    }
    out.push(new URL("../assets/", href));
    return out;
  }

  async function resolveAssetsBase() {
    if (resolvedAssetsBase) return resolvedAssetsBase;
    for (const base of candidateAssetsBases()) {
      const probe = new URL("data/relics.json", base);
      try {
        const res = await fetch(probe, { method: "GET", cache: "no-store" });
        if (res.ok) {
          resolvedAssetsBase = base;
          return base;
        }
      } catch (_) {
        /* try next */
      }
    }
    resolvedAssetsBase = new URL("../assets/", global.location.href);
    return resolvedAssetsBase;
  }

  function relicObjectArtUrl(id, base) {
    return new URL(`textures/relics/${id}_object.png`, base).href;
  }

  let loadGen = 0;

  /**
   * @param {HTMLImageElement} img
   * @param {HTMLElement} hint
   * @param {string} id relic slug
   * @param {(loaded: boolean, detail: string) => void} [onState]
   */
  function loadRelicObjectArt(img, hint, id, onState) {
    const gen = ++loadGen;
    img.classList.remove("is-hidden");
    hint.textContent = "Loading…";
    img.removeAttribute("title");
    img.onload = null;
    img.onerror = null;

    const finish = (ok, detail) => {
      if (gen !== loadGen) return;
      img.onload = null;
      img.onerror = null;
      if (ok) {
        img.classList.remove("is-hidden");
        img.title = detail;
        hint.textContent = `${id}_object.png`;
      } else {
        img.removeAttribute("title");
        img.classList.add("is-hidden");
        hint.textContent = detail;
      }
      if (onState) onState(ok, detail);
    };

    resolveAssetsBase().then((base) => {
      if (gen !== loadGen) return;
      const url = relicObjectArtUrl(id, base);
      img.alt = `Relic object art: ${id}`;
      img.onload = () => finish(true, url);
      img.onerror = () =>
        finish(
          false,
          `Missing object art (${url}). Serve the repo from its root, e.g. python3 -m http.server, then open …/tools/relic_flavor_editor.html`
        );
      img.src = url;
      if (img.complete) {
        if (img.naturalWidth > 0) finish(true, url);
        else finish(false, `Missing object art (${url}).`);
      }
    });
  }

  function resetRelicObjectArt(img, hint) {
    loadGen += 1;
    img.onload = null;
    img.onerror = null;
    img.removeAttribute("src");
    img.removeAttribute("title");
    img.classList.remove("is-hidden");
    hint.textContent = "";
  }

  global.MahjuroEditorAssets = {
    resolveAssetsBase,
    relicObjectArtUrl,
    loadRelicObjectArt,
    resetRelicObjectArt,
  };
})(window);
