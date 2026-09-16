import {useLayoutEffect,useRef} from 'react';

// Keep disclosures mounted; restore the reading position before repainting.
export function KeptPane({active,children,label,className=''}) {
  const element=useRef(null);const position=useRef(0);
  useLayoutEffect(()=>{if(active&&element.current)element.current.scrollTop=position.current;},[active]);
  return <div ref={element} hidden={!active} aria-label={label} className={`todo-kept-pane ${className}`}
    onScroll={event=>{if(active&&event.target===element.current)position.current=event.currentTarget.scrollTop;}}>{children}</div>;
}
