import { useEffect, useRef } from 'react';

export function CategoryTabs({ options, value, onChange, className = '' }) {
  const root = useRef(null);
  const tabs = useRef(new Map());
  useEffect(() => {
    const reveal = () => tabs.current.get(value)?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    reveal();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(reveal);
    observer.observe(root.current);
    return () => observer.disconnect();
  }, [value]);
  useEffect(() => {
    const element = root.current;
    const wheel = (event) => {
      if (event.ctrlKey || element.scrollWidth <= element.clientWidth) return;
      const before = element.scrollLeft;
      element.scrollLeft += Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
      if (element.scrollLeft !== before) event.preventDefault();
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, []);
  const navigate = (event, index) => {
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1
      : event.key === 'ArrowRight' ? (index + 1) % options.length
        : event.key === 'ArrowLeft' ? (index + options.length - 1) % options.length : null;
    if (next === null) return;
    event.preventDefault(); onChange(options[next].value); tabs.current.get(options[next].value)?.focus();
  };
  return <div ref={root} role="tablist" aria-label="导航分类" className={`fleet-category-tabs ${className}`}>
    {options.map((option, index) => <button key={option.value} ref={(element) => {
      if (element) tabs.current.set(option.value, element); else tabs.current.delete(option.value);
    }} type="button" role="tab" aria-selected={value === option.value} tabIndex={value === option.value ? 0 : -1}
      className="fleet-category-tab" onClick={() => onChange(option.value)} onKeyDown={(event) => navigate(event, index)}>{option.label}</button>)}
  </div>;
}
