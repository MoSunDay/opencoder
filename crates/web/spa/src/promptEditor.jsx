// promptEditor.jsx — 引用的 prompts 资源之 soul/how/output 编辑器：从
// CURRENT 版本读取 soul.md|how.md|output.md（缺失 ⇒ 空文本，404 吞掉），
// 「保存」把三份文件一起 PUT /api/agents/resources/prompts/:name（b64）
// 产生新版本并提示版本号；onSaved 回调让外层刷新 meta / 版本列表。

import { Button, Card, Input, Typography, message } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet, apiPut } from './api.js';
import { b64DecodeText, b64EncodeText } from './agentsItems.js';

const { TextArea } = Input;
const { Text } = Typography;

const PROMPT_PARTS = [
  { key: 'soul', file: 'soul.md', label: 'Soul（人格底色）', rows: 5 },
  { key: 'how', file: 'how.md', label: 'How（工作方法）', rows: 9 },
  { key: 'output', file: 'output.md', label: 'Output（产出契约）', rows: 5 },
];

/// 读 CURRENT 版本下的一个 prompt 文件；不存在（404）或解码失败 ⇒ ''。
async function readPromptFile(resourceName, version, file) {
  try {
    const j = await apiGet(
      `/api/agents/resources/prompts/${encodeURIComponent(resourceName)}/versions/${version}/files/${file}`,
    );
    return b64DecodeText(j && j.content_b64);
  } catch {
    return '';
  }
}

export function PromptEditor({ resourceName, onNotice, onSaved }) {
  const [texts, setTexts] = useState({ soul: '', how: '', output: '' });
  const [version, setVersion] = useState(0);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let alive = true;
    if (!resourceName) {
      setTexts({ soul: '', how: '', output: '' });
      setVersion(0);
      return undefined;
    }
    setLoading(true);
    (async () => {
      try {
        const j = await apiGet(`/api/agents/resources/prompts/${encodeURIComponent(resourceName)}/meta`);
        const v = (j && j.meta && j.meta.current) || 0;
        const parts = await Promise.all(PROMPT_PARTS.map((p) => readPromptFile(resourceName, v, p.file)));
        if (!alive) {
          return;
        }
        setVersion(v);
        setTexts({ soul: parts[0], how: parts[1], output: parts[2] });
      } catch (e) {
        if (alive && onNotice) {
          onNotice('读取 prompt 失败: ' + (e && e.message));
        }
      } finally {
        if (alive) {
          setLoading(false);
        }
      }
    })();
    return () => {
      alive = false;
    };
  }, [resourceName, onNotice]);

  if (!resourceName) {
    return <Text type="secondary">未引用 prompt 资源 —— 先在上方选择一个。</Text>;
  }

  const save = async () => {
    setSaving(true);
    try {
      const files = PROMPT_PARTS.map((p) => ({ path: p.file, content_b64: b64EncodeText(texts[p.key]) }));
      const j = await apiPut(`/api/agents/resources/prompts/${encodeURIComponent(resourceName)}`, { files });
      const v = (j && j.version) || version + 1;
      setVersion(v);
      message.success(`已保存，新版本 v${v}`);
      if (onSaved) {
        onSaved(v);
      }
    } catch (e) {
      if (onNotice) {
        onNotice('保存 prompt 失败: ' + (e && e.message));
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card
      size="small"
      title={`Prompt 内容（当前 v${version}）`}
      loading={loading}
      extra={<Button size="small" type="primary" loading={saving} onClick={save}>保存</Button>}
      style={{ marginTop: 12 }}
    >
      {PROMPT_PARTS.map((p) => (
        <div key={p.key} style={{ marginBottom: 12 }}>
          <Text type="secondary" style={{ fontSize: 12 }}>{p.label} · {p.file}</Text>
          <TextArea
            rows={p.rows}
            value={texts[p.key]}
            aria-label={`prompt-${p.key}`}
            onChange={(e) => setTexts({ ...texts, [p.key]: e.target.value })}
          />
        </div>
      ))}
    </Card>
  );
}
