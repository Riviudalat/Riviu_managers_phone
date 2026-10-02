// Narrow CDP/IPC diagnostic client. Never Start/Execute, arbitrary invoke or public input.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { writeFile, mkdir } from 'node:fs/promises';
const require = createRequire(resolve('apps/desktop/package.json'));
const { chromium } = require('playwright');
const args = Object.fromEntries(process.argv.slice(2).reduce((rows,arg,index,all)=>arg.startsWith('--')?[...rows,[arg.slice(2),all[index+1]]]:rows,[]));
const port = Number(args.port);
const mode = args.mode ?? 'status';
if (!Number.isInteger(port) || port<1024 || port>65535 || !['status','metadata','inspect','interaction','publish','run-status','cancel','policy-check','shutdown','reconcile-setup'].includes(mode)) throw Error('Explicit loopback port and valid diagnostic mode required');
const commands = {status:'no_public_status','reconcile-setup':'no_public_reconcile_setup',shutdown:'no_public_shutdown',metadata:'no_public_metadata',inspect:'no_public_inspect',interaction:'no_public_prepare_interaction',publish:'no_public_prepare_publish','run-status':'no_public_run_status',cancel:'no_public_cancel'};
if (!args.report || !resolve(args.report).startsWith(resolve('target')+'/') && !resolve(args.report).startsWith(resolve('target')+'\\')) throw Error('Local target report directory required');
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
try {
    const readyDeadline=Date.now()+15000;
    let pages=[];
    do {
        pages=browser.contexts().flatMap(context=>context.pages()).filter(page=>/^https?:\/\/(127\.0\.0\.1|localhost|tauri\.localhost)(:|\/)/.test(page.url()) || page.url().startsWith('tauri://localhost/'));
        if (pages.length>1) throw Error('Multiple local Tauri pages; no command sent');
        if (pages.length===1) break;
        await new Promise(resolve=>setTimeout(resolve,250));
    } while (Date.now()<readyDeadline);
    if (pages.length!==1) throw Error('Exactly one local Tauri page is required');
    const page=pages[0];
    const status=await page.evaluate(async()=>window.__TAURI_INTERNALS__.invoke('no_public_status'));
    if (status.publicEffectsAllowed!==false || !status.activationId || status.activationId!==args.activation) throw Error('Wrong process activation or missing no-public policy');
    const parameters=mode==='reconcile-setup'?{udid:args.udid,priorActivation:args['prior-activation'],priorRequest:args['prior-request']}:mode==='shutdown'?{activationId:args.activation}:mode==='status'||mode==='policy-check'?{}:mode==='run-status'||mode==='cancel'?{requestId:args.request}:{udid:args.udid,requestId:args.request};
    if(!['status','policy-check','metadata','shutdown','reconcile-setup'].includes(mode)&&(!args.request || !/^[a-f0-9-]{36}$/i.test(args.request)))throw Error('Explicit UUID required');
    if(['metadata','inspect','interaction','publish','reconcile-setup'].includes(mode)&&!status.scopeDeviceIds.includes(args.udid))throw Error('Device outside activation');
    let result;
    if (mode==='policy-check') {
        // Missing parameters can never start a normal handler. Prove policy rejects
        // at ingress, rather than mistaking argument validation for no-public denial.
        result=await page.evaluate(async()=> {
            const denied=[];
            for (const command of ['publish_start','interaction_start_thread','device_type_text','agent_repair']) {
                try { await window.__TAURI_INTERNALS__.invoke(command,{}); throw Error(`Unexpectedly accepted ${command}`); }
                catch (error) {
                    if (error?.code!=='NoPublicRehearsalDenied') throw error;
                    denied.push({command,code:error.code});
                }
            }
            return {publicEffectsAllowed:false,denied};
        });
    } else {
        result=mode==='status'?status:await page.evaluate(async({command,parameters})=> {
            try { return await window.__TAURI_INTERNALS__.invoke(command,parameters); }
            catch (error) { return {diagnosticCommandError:true,code:error?.code??'Unknown',message:error?.message??String(error),publicEffectsAllowed:false}; }
        },{command:commands[mode],parameters});
    }
    // A start is sent once. Waiting polls receipts only, never replays preparation.
    if (args.wait === 'true' && ['inspect','interaction','publish'].includes(mode)) {
        const deadline=Date.now()+360000;
        while (result.state==='running' && Date.now()<deadline) {
            await new Promise(resolve=>setTimeout(resolve,1000));
            result=await page.evaluate(async(requestId)=>window.__TAURI_INTERNALS__.invoke('no_public_run_status',{requestId}),args.request);
            if (!result || result.requestId!==args.request || result.publicEffectsAllowed!==false) throw Error('Invalid diagnostic receipt while waiting');
        }
        if (result.state==='running') throw Error('Receipt wait exhausted; no retry, inspect/cancel by requestId');
    }
    await mkdir(args.report,{recursive:true});
    await writeFile(resolve(args.report,`${mode}-${mode==='metadata'?args.udid:args.request??'status'}.json`),JSON.stringify(result,null,2),{flag:'wx'});
    console.log(JSON.stringify(result));
    if (result.diagnosticCommandError || result.state==='needsAttention' || result.state==='blocked') process.exitCode=1;
} finally { await browser.close(); }
