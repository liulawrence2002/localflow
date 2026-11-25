import { useEffect, useRef } from "react";

/**
 * Run `task` every `intervalMs` while the window is visible, and at once whenever it becomes
 * visible again or gains the focus. Nothing runs while the window is hidden in the tray.
 */
export function usePolling(task: () => void, intervalMs: number): void {
  const taskRef = useRef(task);
  taskRef.current = task;

  useEffect(() => {
    let timer: number | null = null;
    const run = () => taskRef.current();
    const start = () => {
      if (timer === null) {
        timer = window.setInterval(run, intervalMs);
      }
    };
    const stop = () => {
      if (timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        stop();
      } else {
        run();
        start();
      }
    };

    if (document.visibilityState !== "hidden") {
      run();
      start();
    }
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("focus", run);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", run);
    };
  }, [intervalMs]);
}

/** Run `task` when the window becomes visible again or gains the focus. */
export function useOnWindowReturn(task: () => void): void {
  const taskRef = useRef(task);
  taskRef.current = task;

  useEffect(() => {
    const run = () => taskRef.current();
    const onVisibility = () => {
      if (document.visibilityState !== "hidden") {
        run();
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("focus", run);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", run);
    };
  }, []);
}
