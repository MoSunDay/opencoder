// @vitest-environment jsdom
// statusTag DOM contract: the ONE status → (color, 中文) table. Guards the
// copy asserted across the fleet/todo/project DOM suites (running=运行中 /
// done=已完成 / failed=失败 / cancelled=已取消 …) plus the unknown-status
// fallback and the label/color overrides panels rely on (nodes 在线态).

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import '../test/setup-dom.js';
import { STATUS_META, StatusTag, statusColor, statusLabel } from './statusTag.jsx';

const tagClassOf = (text) => screen.getByText(text).className;

afterEach(() => cleanup());

describe('StatusTag rendering', () => {
  it('renders the core execution statuses with their guarded copy', () => {
    render(<div>
      <StatusTag status="running" />
      <StatusTag status="done" />
      <StatusTag status="failed" />
      <StatusTag status="cancelled" />
      <StatusTag status="pending" />
    </div>);
    expect(screen.getByText('运行中')).toBeTruthy();
    expect(screen.getByText('已完成')).toBeTruthy();
    expect(screen.getByText('失败')).toBeTruthy();
    expect(screen.getByText('已取消')).toBeTruthy();
    expect(screen.getByText('等待节点确认')).toBeTruthy();
    expect(tagClassOf('运行中')).toContain('ant-tag-processing');
    expect(tagClassOf('已完成')).toContain('ant-tag-success');
    expect(tagClassOf('失败')).toContain('ant-tag-error');
  });

  it('falls back to default color + the raw status string for unknown statuses', () => {
    render(<StatusTag status="weird_state" />);
    const el = screen.getByText('weird_state');
    expect(el.className).not.toContain('ant-tag-processing');
    expect(el.className).not.toContain('ant-tag-success');
  });

  it('renders "-" for a missing status', () => {
    render(<StatusTag status={undefined} />);
    expect(screen.getByText('-')).toBeTruthy();
  });

  it('lets label and color override the table (nodes 在线态 pattern)', () => {
    render(<StatusTag status="online" label="resource_error" color="error" />);
    const el = screen.getByText('resource_error');
    expect(el.className).toContain('ant-tag-error');
  });
});

describe('pure helpers', () => {
  it('statusColor maps statuses to antd color tokens with default fallback', () => {
    expect(statusColor('running')).toBe('processing');
    expect(statusColor('done')).toBe('success');
    expect(statusColor('failed')).toBe('error');
    expect(statusColor('offline')).toBe('error');
    expect(statusColor('weird')).toBe('default');
    expect(statusColor(undefined)).toBe('default');
  });

  it('statusLabel is Chinese with raw fallback and "-" for missing', () => {
    expect(statusLabel('running')).toBe('运行中');
    expect(statusLabel('suspended')).toBe('已挂起');
    expect(statusLabel('planned')).toBe('已规划');
    expect(statusLabel('draft')).toBe('草稿');
    expect(statusLabel('nope')).toBe('nope');
    expect(statusLabel(undefined)).toBe('-');
  });

  it('every STATUS_META row carries a color and a non-empty label', () => {
    Object.entries(STATUS_META).forEach(([status, meta]) => {
      expect(typeof meta.color).toBe('string');
      expect(meta.color.length).toBeGreaterThan(0);
      expect(typeof meta.label).toBe('string');
      expect(meta.label.length).toBeGreaterThan(0);
      expect(statusColor(status)).toBe(meta.color);
      expect(statusLabel(status)).toBe(meta.label);
    });
  });
});
