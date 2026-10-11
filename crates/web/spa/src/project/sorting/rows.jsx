import { createContext, useContext } from 'react';
import { Button } from 'antd';
import { HolderOutlined } from '@ant-design/icons';
import { DndContext, KeyboardSensor, PointerSensor, closestCenter, useSensor, useSensors } from '@dnd-kit/core';
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';

const Disabled = createContext(false);
const Handle = createContext({});

export function SortableRows({ rows, disabled, onMove, children }) {
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }));
  return <Disabled.Provider value={disabled}>
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={({ active, over }) => {
      if (!disabled && over && active.id !== over.id) onMove(active.id, over.id);
    }}>
      <SortableContext items={rows.map((row) => row.id)} strategy={verticalListSortingStrategy}>{children}</SortableContext>
    </DndContext>
  </Disabled.Provider>;
}

export function SortableRow({ style, ...props }) {
  const disabled = useContext(Disabled);
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } = useSortable({ id: props['data-row-key'], disabled });
  return <Handle.Provider value={{ attributes, listeners, setActivatorNodeRef, disabled }}>
    <tr {...props} ref={setNodeRef} style={{ ...style, transform: CSS.Translate.toString(transform), transition,
      ...(isDragging ? { position: 'relative', zIndex: 2, opacity: 0.7 } : {}) }} />
  </Handle.Provider>;
}

export function RowDragHandle({ title }) {
  const { attributes, listeners, setActivatorNodeRef, disabled } = useContext(Handle);
  return <Button {...attributes} {...listeners} ref={setActivatorNodeRef} disabled={disabled} type="text" size="small"
    className="project-row-drag" aria-label={`拖动专项 ${title}`} title="拖动调整专项顺序" icon={<HolderOutlined />}
    onClick={(event) => event.stopPropagation()} />;
}
