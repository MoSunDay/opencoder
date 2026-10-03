// @vitest-environment jsdom
import '../test/setup-dom.js';
import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { CategoryTabs } from './categoryTabs.jsx';

it('keeps complete labels, scrolls overflowing tabs, and supports keyboard selection', () => {
  const onChange = vi.fn();
  const options = ['项目', 'Agent', '节点', 'Ontology'].map((label) => ({ value: label, label }));
  const view = render(<CategoryTabs options={options} value="项目" onChange={onChange} />);
  const root = screen.getByRole('tablist');
  Object.defineProperties(root, { scrollWidth: { value: 360 }, clientWidth: { value: 180 } });
  fireEvent.wheel(root, { deltaY: 40 });
  expect(root.scrollLeft).toBe(40);
  fireEvent.keyDown(screen.getByRole('tab', { name: '项目' }), { key: 'End' });
  expect(onChange).toHaveBeenLastCalledWith('Ontology');
  expect(document.activeElement.textContent).toBe('Ontology');
  const scroll = vi.spyOn(screen.getByRole('tab', { name: 'Ontology' }), 'scrollIntoView');
  view.rerender(<CategoryTabs options={options} value="Ontology" onChange={onChange} />);
  expect(scroll).toHaveBeenCalledWith({ block: 'nearest', inline: 'nearest' });
  expect(screen.getByRole('tab', { name: 'Ontology' }).getAttribute('aria-selected')).toBe('true');
});

it('keeps the active category visible when the tab bar shrinks', () => {
  let resize;
  const disconnect = vi.fn();
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback) { resize = callback; }
    observe() {}
    disconnect = disconnect;
  });
  try {
    const view = render(<CategoryTabs options={[{ value: 'ontology', label: 'Ontology' }]} value="ontology" onChange={() => {}} />);
    const scroll = vi.spyOn(screen.getByRole('tab', { name: 'Ontology' }), 'scrollIntoView');
    resize();
    expect(scroll).toHaveBeenCalledWith({ block: 'nearest', inline: 'nearest' });
    view.unmount();
    expect(disconnect).toHaveBeenCalledOnce();
  } finally { vi.unstubAllGlobals(); }
});
