import {useState} from 'react';
import {Button, Modal, Typography, Upload} from 'antd';
import {mergeArchive, unzipArchive} from './resourceModel.js';

// 单个压缩包体积上限：解包后的合并集才受 1.5 MiB 资源上限约束，这里只挡
// 明显异常的输入。
const MAX_ARCHIVE_BYTES = 8 * 1024 * 1024;

// 保存按钮右侧的「上传压缩包」：选择 .zip → 解包预览 → 对话框说明覆盖语义
// （同名文件直接覆盖，只覆盖文件不动目录，目录内其他文件保留）→ 确认后合并
// 进草稿，由「保存」统一提交。错误统一经 onError 冒泡到工具栏下方的 Alert。
export function ArchiveUpload({cat,files,disabled,onMerge,onError}) {
  const [pending,setPending] = useState(null);
  const [busy,setBusy] = useState(false);
  // 始终 resolve false：否则 antd 视为「继续上传」，会在没有 action 的情况下
  // 发出一次后台请求。本组件完全自管解包与合并，默认上传必须被阻止。
  const pick = async file => {
    if (!file) return false;
    onError('');
    setPending(null);
    if (!/\.zip$/i.test(file.name)) { onError('请选择 .zip 压缩包'); return false; }
    if (file.size > MAX_ARCHIVE_BYTES) { onError(`压缩包超过 ${MAX_ARCHIVE_BYTES / 1024 / 1024} MiB`); return false; }
    setBusy(true);
    try {
      const {files: incoming,skipped} = await unzipArchive(file);
      if (!incoming.length) throw new Error('压缩包中没有可导入的文件');
      const merged = mergeArchive(cat,files,incoming);
      const overwritten = incoming.filter(item => Object.hasOwn(files,item.path)).length;
      setPending({merged,added: incoming.length - overwritten,overwritten,skipped,paths: incoming.map(item => item.path)});
    } catch (error) { onError(error.message); }
    finally { setBusy(false); }
    return false;
  };
  const confirm = () => { onMerge(pending.merged); setPending(null); };
  return <>
    <Upload accept=".zip" showUploadList={false} disabled={disabled || busy} beforeUpload={pick}>
      <Button disabled={disabled || busy}>上传压缩包</Button>
    </Upload>
    <Modal open={!!pending} title="覆盖上传" okText="覆盖上传" cancelText="取消" onOk={confirm} onCancel={() => setPending(null)}>
      <p>压缩包内共 {pending?.paths.length ?? 0} 个文件：新增 {pending?.added ?? 0} 个，覆盖同名 {pending?.overwritten ?? 0} 个。</p>
      <p>同名文件将被直接覆盖；只覆盖同名文件，不会删除或清空目录，目录中原有的其他文件全部保留。</p>
      <p><Typography.Text type="secondary">导入后需点击「保存」才会生效。</Typography.Text></p>
      <div style={{maxHeight:160,overflow:'auto',background:'#fafafa',border:'1px solid #f0f0f0',borderRadius:4,padding:'4px 8px',fontSize:12}}>
        {pending?.paths.map(path => <div key={path}>{path}</div>)}
      </div>
      {!!pending?.skipped && <p><Typography.Text type="secondary">已忽略 {pending.skipped} 个不受支持的条目（隐藏文件 / 系统元数据）。</Typography.Text></p>}
    </Modal>
  </>;
}
