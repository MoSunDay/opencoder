// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import '../test/setup-dom.js';
import { DefEditor } from '../dag/defEditor.jsx';
const DEF = { id: 'd', name: 'etl', spec: { name: 'etl', steps: [
  { name: 'fetch', kind: { type: 'wasm', command: 't.wasm' } },
  { name: 'review', kind: { type: 'agent', prompt: 'r' } },
] } };
describe('dbg', () => {
  it('two clicks', async () => {
    render(<DefEditor open def={DEF} saving={false} onClose={vi.fn()} onSave={vi.fn()} />);
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(2));
    fireEvent.click(screen.getByRole('button', { name: /连线/ }));
    fireEvent.click(document.querySelector('[data-id="review"] .dag-edit-node'));
    await waitFor(() => expect(document.querySelector('[data-id="review"] .dag-edit-node').className).toContain('dag-edit-node--linksrc'));
    fireEvent.click(document.querySelector('[data-id="fetch"] .dag-edit-node'));
    console.log('EDGES:', document.querySelectorAll('.react-flow__edge').length);
    console.log('FETCHCLS:', document.querySelector('[data-id="fetch"] .dag-edit-node').className);
    console.log('BAR:', (document.querySelector('.dag-edit-linkbar') || {}).textContent || 'none');
    console.log('FETCHSEL:', document.querySelector('[data-id="fetch"]').className);
    expect(true).toBe(true);
  });
});
