import {mkdir,readFile,open,rename} from 'node:fs/promises';
import {dirname} from 'node:path';
import {initial} from './state.mjs';
export async function atomic(file,value){
 await mkdir(dirname(file),{recursive:true,mode:0o700});
 const temporary=`${file}.${process.pid}.tmp`,handle=await open(temporary,'w',0o600);
 try{await handle.writeFile(JSON.stringify(value,null,2)+'\n');await handle.sync();}finally{await handle.close();}
 await rename(temporary,file);const dir=await open(dirname(file),'r');try{await dir.sync();}finally{await dir.close();}
}
export async function openStore(file){
 let state;
 try{state=JSON.parse(await readFile(file,'utf8'));}catch(error){if(error.code!=='ENOENT')throw error;state=initial();await atomic(file,state);}
 if(state.version!==1)throw Error('Unsupported state version');
 let queue=Promise.resolve();
 return {read:()=>structuredClone(state),transaction(fn){
  const work=queue.then(async()=>{const next=structuredClone(state),result=await fn(next);await atomic(file,next);state=next;return structuredClone(result);});
  queue=work.catch(()=>{});return work;
 }};
}