import { useEvent } from './ui/editing/useEvent.js';
// promptEditor.jsx — 引用的 prompts 资源之 soul/how/output 编辑器：从
// CURRENT 版本读取 soul.md|how.md|output.md（缺失 ⇒ 空文本，404 吞掉），
// 「保存」把三份文件一起 PUT /api/agents/resources/prompts/:name（b64）
// 产生新版本并提示版本号；onSaved 回调让外层刷新 meta / 版本列表。

import { Alert, Button, Card, Input, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet, apiPut } from './api.js';
import { b64EncodeText } from './agentsItems.js';
import { err } from './notice.js';
import { useMessage } from './ui/appMessage.js';

const { TextArea } = Input;
const { Text } = Typography;

const PROMPT_PARTS = [
  { key: 'soul', file: 'soul.md', label: 'Soul（人格底色）', rows: 5 },
  { key: 'how', file: 'how.md', label: 'How（工作方法）', rows: 9 },
  { key: 'output', file: 'output.md', label: 'Output（产出契约）', rows: 5 },
];

/// 读 CURRENT 版本下的一个 prompt 文件；不存在（404）⇒ ''。
async function readPromptFile(resourceName, version, file) {
  try {
    const j = await apiGet(
      `/api/agents/resources/prompts/${encodeURIComponent(resourceName)}/versions/${version}/files/${file}`,
    );
    if (typeof j?.content_b64 !== 'string') throw new Error('prompt 文件响应缺少正文');
    const bytes = Uint8Array.from(atob(j.content_b64), (char) => char.charCodeAt(0));
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch (e) {
    if (e.status === 404) return '';
    throw e;
  }
}

function PromptEditorSession({ resourceName, onNotice: noticeCallback, onSaved }) {
  const onNotice = useEvent(noticeCallback);
  const msg = useMessage();
  const [texts, setTexts] = useState({ soul: '', how: '', output: '' });
  const [loadError, setLoadError] = useState('');
  const [loaded, setLoaded] = useState(false);
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
    setLoading(true); setLoaded(false); setLoadError('');
    (async () => {
      try {
        const j = await apiGet(`/api/agents/resources/prompts/${encodeURIComponent(resourceName)}/meta`);
        const v = (j && j.meta && j.meta.current) || 0;
        const parts = await Promise.all(PROMPT_PARTS.map((p) => readPromptFile(resourceName, v, p.file)));
        if (!alive) {
          return;
        }
        setLoaded(true);
        setVersion(v);
        setTexts({ soul: parts[0], how: parts[1], output: parts[2] });
      } catch (e) {
        if (alive && onNotice) {
          setLoadError('读取 prompt 失败: ' + e.message);
          onNotice(err('读取 prompt 失败: ' + e.message));
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
    if (saving || !loaded) return;
    setSaving(true);
    try {
      const files = PROMPT_PARTS.map((p) => ({ path: p.file, content_b64: b64EncodeText(texts[p.key]) }));
      const j = await apiPut(`/api/agents/resources/prompts/${encodeURIComponent(resourceName)}`, { files });
      const v = (j && j.version) || version + 1;
      setVersion(v);
      msg.success(`已保存，新版本 v${v}`);
      if (onSaved) {
        onSaved(v);
      }
    } catch (e) {
      if (onNotice) {
        onNotice(err('保存 prompt 失败: ' + (e && e.message)));
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
      extra={<Button size="small" type="primary" loading={saving} disabled={!loaded} onClick={save}>保存</Button>}
      style={{ marginTop: 12 }}
    >
      {loadError && <Alert type="error" showIcon title={loadError} />}
      {PROMPT_PARTS.map((p) => (
        <div key={p.key} style={{ marginBottom: 12 }}>
          <Text type="secondary" style={{ fontSize: 12 }}>{p.label} · {p.file}</Text>
          <TextArea
            disabled={saving || !loaded}
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

export function PromptEditor(props) {
  return <PromptEditorSession key={props.resourceName || "empty"} {...props} />;
}
