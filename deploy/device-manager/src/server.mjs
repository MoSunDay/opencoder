import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import {openStore} from './store.mjs';
import {service} from './service.mjs';
import {remote} from './remote.mjs';
export async function start(config){
 const token=(await readFile(config.token_file,'utf8')).trim();if(token.length<32)throw Error('Authentication token must contain at least 32 characters');
 if(!Number.isInteger(config.port)||config.port<1024||!config.listen_host)throw Error('Explicit listener configuration required');
 const store=await openStore(config.state_file),api=service(config,store,remote(config),token);
 const server=createServer(async(req,res)=>{
  try{
   const auth=req.headers.authorization||'';if(!auth.startsWith('Bearer ')||!api.authorized(auth.slice(7)))throw Object.assign(Error('Authentication required'),{status:401});
   let body='';for await(const chunk of req){body+=chunk;if(Buffer.byteLength(body)>65536)throw Object.assign(Error('Request too large'),{status:413});}
   const input=body?JSON.parse(body):{},value=await api.handle(req.method,new URL(req.url,'http://localhost').pathname,input,auth.slice(7));
   res.writeHead(200,{'content-type':'application/json','cache-control':'no-store'});res.end(JSON.stringify(value));
  }catch(error){
   const safe=String(error.code||'').startsWith('upstream_')?{code:error.code,upstream_status:error.upstream_status,cause_code:error.cause_code,attempts:error.attempts}:{};
   if(!error.status)process.stderr.write(JSON.stringify({event:'device_api_error',method:req.method,path:new URL(req.url,'http://localhost').pathname,...safe})+'\n');
   res.writeHead(error.status??(error instanceof SyntaxError?400:503),{'content-type':'application/json','cache-control':'no-store'});
   res.end(JSON.stringify({error:error.status?error.message:'Operation failed; reservation retained',...safe}));
  }
 });
 await new Promise((yes,no)=>{server.once('error',no);server.listen(config.port,config.listen_host,yes);});
 let stopped=false,timer;
 async function tick(){if(stopped)return;try{await api.poll();}catch(error){process.stderr.write('Device reconciliation failed; reservations retained\n');}if(!stopped)timer=setTimeout(tick,config.poll_ms??5000);}
 timer=setTimeout(tick,config.poll_ms??5000);
 return {server,store,async close(){stopped=true;clearTimeout(timer);await new Promise(resolve=>server.close(resolve));}};
}
if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href){
 const file=process.argv[2];if(!file)throw Error('Usage: node src/server.mjs CONFIG.json');
 const instance=await start(JSON.parse(await readFile(file,'utf8')));
 let closing=false;for(const signal of ['SIGINT','SIGTERM'])process.on(signal,async()=>{if(closing)return;closing=true;await instance.close();});
}
