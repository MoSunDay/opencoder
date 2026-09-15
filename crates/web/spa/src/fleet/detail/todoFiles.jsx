import { TodoWorkbench } from '../../todo/review/workbench.jsx';

export function TodoRunEmbed({ id }) {
  return <TodoWorkbench key={id} id={id} showControls={false} />;
}
