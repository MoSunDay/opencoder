import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** Size the workspace, while the canvas itself takes the remaining flex space. */
export function useViewportHeight() {
  const ref = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(600);
  const measure = useCallback(() => {
    if (!ref.current) return;
    const container = ref.current.closest(".fleet-content");
    const bottom = container ? parseFloat(getComputedStyle(container).paddingBottom) : 20;
    setHeight(Math.max(160, Math.floor(window.innerHeight - ref.current.getBoundingClientRect().top - bottom)));
  }, []);
  useLayoutEffect(() => { measure(); });
  useEffect(() => {
    const observer = new ResizeObserver(measure);
    if (ref.current?.parentElement) observer.observe(ref.current.parentElement);
    window.addEventListener("resize", measure);
    return () => { observer.disconnect(); window.removeEventListener("resize", measure); };
  }, [measure]);
  return { ref, height };
}
