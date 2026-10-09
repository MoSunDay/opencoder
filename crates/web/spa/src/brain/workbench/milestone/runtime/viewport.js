import { useLayoutEffect, useRef, useState } from 'react';

export function useRunViewport(fullscreen) {
  const ref = useRef(null);
  const [height, setHeight] = useState(null);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return undefined;
    const update = () => setHeight(Math.max(320,
      (window.visualViewport?.height || window.innerHeight) - element.getBoundingClientRect().top - 16));
    const observer = new ResizeObserver(update);
    observer.observe(element.parentElement);
    window.addEventListener('resize', update);
    window.visualViewport?.addEventListener('resize', update);
    update();
    return () => {
      observer.disconnect(); window.removeEventListener('resize', update);
      window.visualViewport?.removeEventListener('resize', update);
    };
  }, [fullscreen]);
  return { ref, style: fullscreen ? undefined : { height: height ?? 'calc(100dvh - 180px)' } };
}
