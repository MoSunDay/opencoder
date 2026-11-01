// Pure-function tests for the auto side of the live ladder disclosure
// (TUI chat_step_live.rs parity): while a turn runs the open slot is the
// turn row + the LAST step + that step's calls aggregate; idle opens
// nothing. Override/`isOpen` behavior is covered end-to-end by
// stepsBlock.dom.test.jsx through TranscriptView.

import { describe, expect, it } from 'vitest';
import { autoOpenKeys } from './useLadderDisclosure.js';

describe('autoOpenKeys', () => {
  it('opens the turn row plus the LAST step and its calls while running', () => {
    expect(autoOpenKeys(true, 1)).toEqual(['steps', 'step:1', 'calls:1']);
    expect(autoOpenKeys(true, 2)).toEqual(['steps', 'step:2', 'calls:2']);
    expect(autoOpenKeys(true, 5)).toEqual(['steps', 'step:5', 'calls:5']);
  });

  it('a step born while running moves the auto slot, so the old step folds', () => {
    expect(autoOpenKeys(true, 1)).not.toContain('step:2');
    expect(autoOpenKeys(true, 2)).not.toContain('step:1');
    expect(autoOpenKeys(true, 2)).not.toContain('calls:1');
  });

  it('opens nothing once the turn settles', () => {
    expect(autoOpenKeys(false, 3)).toEqual([]);
    expect(autoOpenKeys(false, 0)).toEqual([]);
  });

  it('opens nothing without steps, even while running', () => {
    expect(autoOpenKeys(true, 0)).toEqual([]);
  });

  it('key naming mirrors the Collapse item keys in stepsBlock.jsx', () => {
    const keys = autoOpenKeys(true, 7);
    expect(keys.every((k) => typeof k === 'string')).toBe(true);
    expect(keys).toHaveLength(3);
  });
});
