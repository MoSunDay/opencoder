// @vitest-environment jsdom
// Markdown sanitizer gate (review P2): markdown.jsx is the SPA's ONLY
// dangerouslySetInnerHTML sink, so its DOMPurify wrapping gets its own
// focused suite — (a) script payloads are stripped before they reach the
// DOM, (b) legit GFM (bold / table) still renders, (c) empty text keeps the
// em-dash placeholder convention. Same contract style as project.dom.test.

import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/react';

import '../test/setup-dom.js';
import { Markdown } from './markdown.jsx';

describe('Markdown DOMPurify sanitization', () => {
  it('strips <script> payloads from the rendered HTML', () => {
    const { container } = render(
      <Markdown text={'before <script>alert(1)</script> after\n\n<img src=x onerror="alert(2)">'} />,
    );
    const html = container.innerHTML;
    expect(html).not.toContain('<script');
    expect(html).not.toContain('alert(1)');
    expect(html).not.toContain('onerror');
    // surrounding legit content survives the strip.
    expect(html).toContain('before');
    expect(html).toContain('after');
  });

  it('still renders normal GFM (bold and tables)', () => {
    const bold = render(<Markdown text={'**bold** text'} />);
    expect(bold.container.innerHTML).toContain('<strong>bold</strong>');

    const table = render(
      <Markdown text={'| 甲 | 乙 |\n| --- | --- |\n| 1 | 2 |'} />,
    );
    const html = table.container.innerHTML;
    expect(html).toContain('<table>');
    expect(html).toContain('<th>甲</th>');
    expect(html).toContain('<td>1</td>');
  });

  it('renders the em-dash placeholder for empty/absent text', () => {
    const empty = render(<Markdown text="" />);
    expect(empty.container.querySelector('.md-body--empty')).not.toBeNull();
    expect(empty.container.textContent).toBe('—');

    const absent = render(<Markdown text={null} />);
    expect(absent.container.querySelector('.md-body--empty')).not.toBeNull();
    expect(absent.container.textContent).toBe('—');
  });
});
