// Trunk initializer (see index.html, data-initializer): drives the loading
// screen while the game's wasm downloads and starts. The game itself removes
// the screen (#tbs-loading) once the first mission is on screen; until then
// it shows the stage text the game sets.
export default function loader() {
  const $ = (id) => document.getElementById(id);
  const mb = (n) => (n / 1048576).toFixed(1);
  const set = (text, fraction) => {
    const status = $("tbs-loading-status");
    const bar = $("tbs-loading-bar");
    if (status) status.textContent = text;
    if (bar) {
      bar.classList.toggle("indeterminate", fraction == null);
      bar.style.width = fraction == null ? "" : `${Math.round(fraction * 100)}%`;
    }
  };
  return {
    onStart: () => set("Downloading the game...", 0),
    onProgress: ({ current, total }) => {
      if (total > 0) set(`Downloading the game... ${mb(current)} / ${mb(total)} MB`, Math.min(current / total, 1));
      else set(`Downloading the game... ${mb(current)} MB`, null);
    },
    onComplete: () => set("Starting the engine...", null),
    onSuccess: () => {},
    onFailure: (error) => {
      // Bevy's web event loop "throws" to hand control to the browser: not an error.
      if (String(error).includes("Using exceptions for control flow")) return;
      set(`Couldn't start the game: ${error}`, 0);
      $("tbs-loading")?.classList.add("failed");
      console.error(error);
    },
  };
}
