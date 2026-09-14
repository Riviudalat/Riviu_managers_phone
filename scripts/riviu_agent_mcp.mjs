#!/usr/bin/env node
// MCP adapter uses the running Riviu backend, so it shares device ownership and traces.
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
const require=createRequire(resolve(fileURLToPath(new URL('../apps/desktop/package.json',import.meta.url))));
const {Server}=require('@modelcontextprotocol/sdk/server/index.js');
const {StdioServerTransport}=require('@modelcontextprotocol/sdk/server/stdio.js');
const {ListToolsRequestSchema,CallToolRequestSchema}=require('@modelcontextprotocol/sdk/types.js');
const endpoint=process.env.RIVIU_API_URL??'http://127.0.0.1:22222';
const token=process.env.RIVIU_API_TOKEN;
if(!token)throw Error('Set RIVIU_API_TOKEN from Riviu API settings');
const url=new URL(endpoint);if(!['127.0.0.1','localhost','[::1]'].includes(url.hostname))throw Error('Riviu agent endpoint must be local');
const server=new Server({name:'riviu-agent',version:'1.0.0'},{capabilities:{tools:{}}});
const tools=[
 {name:'riviu_devices',description:'List real devices managed by Riviu',inputSchema:{type:'object',properties:{}}},
 ...['observe','tap','record','recording'].map(operation=>({
  name:'riviu_'+operation,
  description:({observe:'Fresh screenshot and inspectable UI elements',tap:'Resolve one selector, tap once and record observed result',record:'Start or stop persistent device recording',recording:'Read recorded steps and outcomes'})[operation],
  inputSchema:{type:'object',required:['udid',...(operation==='tap'?['selector']:operation==='record'?['active']:[])],properties:{
   udid:{type:'string'},
   ...(operation==='tap'?{selector:{type:'object',required:['package'],properties:{package:{type:'string'},text:{type:'string'},description:{type:'string'},resourceId:{type:'string'},className:{type:'string'}}}}:{}),
   ...(operation==='record'?{active:{type:'boolean'},name:{type:'string'}}:{})
  }}
 }))
];
server.setRequestHandler(ListToolsRequestSchema,async()=>({tools}));
server.setRequestHandler(CallToolRequestSchema,async({params})=>{
 if(!tools.some(t=>t.name===params.name))throw Error('Unknown tool');
 const operation=params.name.slice(6);const devices=operation==='devices';
 const response=await fetch(endpoint+(devices?'/v1/devices':'/v1/inspector/'+operation),{method:devices?'GET':'POST',headers:{Authorization:'Bearer '+token,'Content-Type':'application/json'},...(devices?{}:{body:JSON.stringify(params.arguments??{})}),signal:AbortSignal.timeout(120000)});
 const result=await response.json();
 const snapshot=result.result??result;
 const content=[];
 if(snapshot.pngBase64){content.push({type:'image',data:snapshot.pngBase64,mimeType:'image/png'});delete snapshot.pngBase64;}
 content.push({type:'text',text:JSON.stringify(snapshot)});
 return {content,isError:!response.ok};
});
await server.connect(new StdioServerTransport());
