import {useState} from 'react';
import {Alert, Input, Modal} from 'antd';
import {parentPath} from './operations.js';

export function EntryDialog({operation, onApply, onClose}) {
  const {action,path,isDirectory} = operation;
  const creating = action.startsWith('create-');
  const kind = (creating ? action === 'create-directory' : isDirectory) ? '目录' : '文件';
  const title = `${creating ? '新增' : action === 'rename' ? '重命名' : '删除'}${kind}`;
  const [name,setName] = useState(action === 'rename' ? path.split('/').pop() : '');
  const [error,setError] = useState('');
  const apply = () => {
    try {onApply(name);onClose();}
    catch (failure) {setError(failure.message);}
  };
  return <Modal title={title} open onCancel={onClose} onOk={apply}
    okText={action === 'delete' ? '删除' : '确定'} cancelText="取消" okButtonProps={{danger:action === 'delete'}}>
    {action === 'delete' ? <p>删除{kind}「{path}」{isDirectory ? '及其全部文件和子目录' : ''}？</p> : <>
      <p>所在目录：{(creating && isDirectory ? path : parentPath(path)) || '工作区根目录'}</p>
      <Input autoFocus aria-label={`${kind}名称`} value={name} status={error ? 'error' : undefined}
        onFocus={event=>event.target.select()}
        onChange={event => {setName(event.target.value);setError('');}} onPressEnter={apply}/>
    </>}
    {error && <Alert type="error" title={error} style={{marginTop:12}}/>}
  </Modal>;
}
