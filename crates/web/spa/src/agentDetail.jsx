// agentDetail.jsx — 单个 agent 卡片详情：头部（返回 / 名称 / 设为生效 /
// 刷新）+ 五个 tab。四个资源 tab 共用 ResourceRefTab：引用 Select 变更 →
// PUT /api/agents/:name 整卡 current；版本 Select +「回滚」→ POST
// .../rollback。Prompt tab 内嵌 promptEditor（soul/how/output 三文件版本
// 化保存）；Tools 只读展示 references.tools 名称 —— 后端没有逐文件列表
// 端点，上传 UI 留空（缺口已记录，不做）。Meta tab 渲染 history 时间线 +
// references 解析快照。

import {
  Button, Card, Select, Space, Tabs, Tag, Timeline, Typography, message,
} from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet, apiPatch, apiPost, apiPut } from './api.js';
import { REF_FIELDS, resolvedNames, resourceOptions, versionOptions } from './agentsItems.js';
import { PromptEditor } from './promptEditor.jsx';

const { Text, Title } = Typography;

/// 单个资源类别的引用面板：卡片的 field 引用（Select，PUT 整卡）+ 池内
/// 版本（Select）与回滚按钮 + references 只读快照 tag。children 是该类
/// 特有的内容查看器（Prompt 的三文件编辑器）。
function ResourceRefTab({ field, cat, label, meta, resources, onNotice, onCardSaved, children }) {
  const refs = (meta && meta.current) || {};
  const referenced = refs[field] || '';
  const entry = (resources[cat] || []).find((r) => r && r.name === referenced) || null;
  const [rollbackV, setRollbackV] = useState(undefined);

  const changeRef = async (v) => {
    try {
      await apiPut(`/api/agents/${encodeURIComponent(meta.name)}`, {
        current: { ...refs, [field]: v || null },
      });
      message.success('引用已更新');
      onCardSaved();
    } catch (e) {
      if (onNotice) {
        onNotice('更新引用失败: ' + (e && e.message));
      }
    }
  };

  const rollback = async () => {
    if (!rollbackV || !referenced) {
      return;
    }
    try {
      const j = await apiPost(
        `/api/agents/resources/${cat}/${encodeURIComponent(referenced)}/rollback`,
        { version: rollbackV },
      );
      message.success(`已回滚到 v${(j && j.current) || rollbackV}`);
      onCardSaved();
    } catch (e) {
      if (onNotice) {
        onNotice('回滚失败: ' + (e && e.message));
      }
    }
  };

  const names = resolvedNames(meta && meta.references, field);
  return (
    <div>
      <Space wrap style={{ marginBottom: 8 }}>
        <Select
          allowClear
          placeholder={`不引用 ${label}`}
          style={{ minWidth: 240 }}
          value={referenced || undefined}
          onChange={changeRef}
          options={resourceOptions(resources[cat])}
          aria-label={`ref-select-${field}`}
        />
        {referenced ? (
          <>
            <Select
              size="small"
              style={{ minWidth: 110 }}
              placeholder="回滚版本"
              value={rollbackV}
              onChange={setRollbackV}
              options={versionOptions(entry ? entry.versions : [])}
              aria-label={`rollback-version-${field}`}
            />
            <Button size="small" disabled={!rollbackV} onClick={rollback}>回滚</Button>
            <Text type="secondary" style={{ fontSize: 12 }}>
              当前 v{(entry && entry.current) || '?'}
            </Text>
          </>
        ) : null}
      </Space>
      <div style={{ marginBottom: 4 }}>
        {names.length > 0
          ? names.map((n) => <Tag key={n} aria-label={`resolved-${field}`}>{n}</Tag>)
          : <Text type="secondary" style={{ fontSize: 12 }}>暂无解析快照（references 为空或未引用）</Text>}
      </div>
      {children}
    </div>
  );
}

/// Meta tab：引用变更历史（field / from → to / at）+ references 汇总。
function MetaTab({ meta }) {
  const hist = (meta && meta.history) || [];
  const refs = (meta && meta.references) || {};
  return (
    <div>
      <Card size="small" title="引用变更历史">
        {hist.length === 0 ? <Text type="secondary">暂无变更</Text> : (
          <Timeline
            items={hist.map((h, i) => ({
              key: String(i),
              children: (
                <Space wrap size={4}>
                  <Tag>{h.field || '-'}</Tag>
                  <Text style={{ fontSize: 12 }}>{h.from || '—'} → {h.to || '—'}</Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>{h.at || '-'}</Text>
                </Space>
              ),
            }))}
          />
        )}
      </Card>
      <Card size="small" title="解析快照（references）" style={{ marginTop: 12 }}>
        {REF_FIELDS.map(({ field, cat }) => (
          <div key={field} style={{ marginBottom: 4 }}>
            <Text type="secondary" style={{ fontSize: 12, width: 64, display: 'inline-block' }}>{field}</Text>
            {resolvedNames(refs, field).map((n) => <Tag key={n}>{n}</Tag>)}
            {resolvedNames(refs, field).length === 0 ? <Text type="secondary">—</Text> : null}
          </div>
        ))}
      </Card>
    </div>
  );
}

export function AgentDetail({ name, resources, onNotice, onChanged, onBack }) {
  const [meta, setMeta] = useState(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const j = await apiGet(`/api/agents/${encodeURIComponent(name)}/meta`);
      setMeta((j && j.meta) || null);
    } catch (e) {
      if (onNotice) {
        onNotice('获取 agent 详情失败: ' + (e && e.message));
      }
    } finally {
      setLoading(false);
    }
  }, [name, onNotice]);

  useEffect(() => {
    load();
  }, [load]);

  const onCardSaved = useCallback(() => {
    load();
    if (onChanged) {
      onChanged();
    }
  }, [load, onChanged]);

  const activate = async () => {
    try {
      await apiPatch('/api/agents/active', { active: name });
      message.success(`已激活 ${name}`);
    } catch (e) {
      // 400/404 = prompt 预检失败等，服务端 error 字段已并入 e.message
      if (onNotice) {
        onNotice('激活失败: ' + (e && e.message));
      }
    }
  };

  if (loading && !meta) {
    return <Card size="small"><Text type="secondary">加载中…</Text></Card>;
  }
  if (!meta) {
    return <Card size="small"><Text type="danger">卡片读取失败</Text></Card>;
  }
  const refs = meta.current || {};
  const tabProps = { meta, resources, onNotice, onCardSaved };
  return (
    <div>
      <Space style={{ marginBottom: 16 }}>
        <Button size="small" onClick={onBack}>返回</Button>
        <Title level={5} style={{ margin: 0 }}>Agent: {name}</Title>
        <Button size="small" type="primary" onClick={activate}>设为生效</Button>
        <Button size="small" onClick={load}>刷新</Button>
      </Space>
      <Tabs
        defaultActiveKey="prompt"
        items={[
          {
            key: 'prompt',
            label: 'Prompt',
            children: (
              <ResourceRefTab field="prompt" cat="prompts" label="Prompt" {...tabProps}>
                <PromptEditor
                  resourceName={refs.prompt || ''}
                  onNotice={onNotice}
                  onSaved={onCardSaved}
                />
              </ResourceRefTab>
            ),
          },
          {
            key: 'skills',
            label: 'Skills',
            children: <ResourceRefTab field="skills" cat="skills" label="Skills" {...tabProps} />,
          },
          {
            key: 'tools',
            label: 'Tools',
            children: (
              <ResourceRefTab field="tools" cat="tools" label="Tools" {...tabProps}>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  工具随 tools 资源版本整体引用（上表为解析出的工具名）；暂无逐文件列表/上传端点，只读。
                </Text>
              </ResourceRefTab>
            ),
          },
          {
            key: 'memory',
            label: 'Memory',
            children: <ResourceRefTab field="memory" cat="memory" label="Memory" {...tabProps} />,
          },
          { key: 'meta', label: 'Meta', children: <MetaTab meta={meta} /> },
        ]}
      />
    </div>
  );
}
