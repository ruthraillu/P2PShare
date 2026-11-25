import {Miniflare} from 'miniflare';
import {fileURLToPath} from 'node:url';
import {writeFileSync,existsSync,unlinkSync} from 'node:fs';
const root=new URL('../../../.build/cloudflare-native/',import.meta.url);
const restart=fileURLToPath(new URL('restart',root));
// This is an owned test marker, never a user artifact.
if(existsSync(restart)) unlinkSync(restart);
let mf,port=0,generation=0;
async function start(){
 mf=new Miniflare({modules:true,scriptPath:fileURLToPath(new URL('../src/index.js',import.meta.url)),compatibilityDate:'2026-08-01',durableObjects:{ROOMS:{className:'Room',useSQLite:true}},durableObjectsPersist:fileURLToPath(new URL('state',root)),port});
 const url=await mf.ready;port=Number(url.port);generation++;
 writeFileSync(fileURLToPath(new URL('server.json',root)),JSON.stringify({server:url.origin.replace('http:','ws:'),generation}));
 console.log('Local Worker ready at '+url.origin);
}
await start();
let busy=false;
const timer=setInterval(async()=>{
 if(busy||!existsSync(restart))return;
 busy=true;unlinkSync(restart);
 try{await mf.dispose();await start();}catch(error){console.error(error);process.exit(1)}finally{busy=false;}
},100);
process.on('SIGTERM',async()=>{clearInterval(timer);await mf.dispose();process.exit(0)});
