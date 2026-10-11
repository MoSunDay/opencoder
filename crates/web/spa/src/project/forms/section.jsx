import { useId } from 'react';
import './todo.css';

export function TodoSection({ title, children, className = '' }) {
  const id = useId();
  return <section className={`project-todo-section ${className}`} aria-labelledby={id}>
    <h3 id={id}>{title}</h3>
    <div className="project-todo-field-grid">{children}</div>
  </section>;
}

export function TodoActions({ children }) {
  return <div className="project-todo-actions">{children}</div>;
}
