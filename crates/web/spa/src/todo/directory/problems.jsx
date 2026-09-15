import {Button, Modal} from 'antd';

export function FileProblems({problems, onClose, onLocate}) {
  return <Modal title="文件不符合 TODO 框架要求" open={!!problems?.length} onCancel={onClose}
    footer={<Button type="primary" onClick={onClose}>继续修改</Button>}>
    <ul className="todo-file-errors">{(problems || []).map((problem,index) => <li key={`${problem.path}:${index}`}>
      <Button type="link" onClick={() => {onLocate?.(problem);onClose();}}>{problem.path}:{problem.line || 1}:{problem.column || 1}</Button>
      <div>{problem.message}</div>
    </li>)}</ul>
  </Modal>;
}

export function errorProblems(error, path='workflow.json') {
  return error?.body?.diagnostics?.length ? error.body.diagnostics : [{path,message:error.message || String(error),line:1,column:1}];
}
