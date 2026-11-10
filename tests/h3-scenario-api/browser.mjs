// The provider implementation is compiled verbatim from its pinned upstream source.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {pathToFileURL} from 'node:url';
const {RestWesProvider}=await import(pathToFileURL(process.env.GA4GH_WES_PROVIDER_JS).href);
const base=process.env.GA4GH_SCENARIO_API;
const fixture=fs.readFileSync(new URL('../../testbed/pack/fixture.cwl',import.meta.url));
const s=await (await fetch(base+'/control/v1/sessions',{method:'POST',body:JSON.stringify({seed:17})})).json();
const headers={Authorization:'Bearer '+s.token};
const control=async body=>{const r=await fetch(`${base}/control/v1/sessions/${s.session_id}/events`,{method:'POST',headers,body:JSON.stringify(body)});assert.equal(r.status,200);};
class AuthenticatedProvider extends RestWesProvider {
  fetch(input, init={}) {return fetch(input,{...init,headers:{...init.headers,...headers}});}
}
const provider=new AuthenticatedProvider(base+s.base_path+'/wes/v1');
assert.deepEqual((await provider.getServiceInfo()).supported_wes_versions,['1.1.0']);
const req=()=>({workflow_params:{message:'synthetic'},workflow_type:'CWL',workflow_type_version:'v1.2',workflow_url:'fixture.cwl',workflow_attachment:[new File([fixture],'fixture.cwl')]});
const a=await provider.runWorkflow(req());
const b=await provider.runWorkflow(req());
const page=await provider.listRuns(1);assert.equal(page.runs[0].run_id,a.run_id);
const page2=await provider.listRuns(1,page.next_page_token);assert.equal(page2.runs[0].run_id,b.run_id);
await control({action:'advance'});assert.equal((await provider.getRunStatus(a.run_id)).state,'RUNNING');
await control({action:'fault',operation:'wes.cancel',name:'cancel-completion-race'});
await provider.cancelRun(a.run_id);assert.equal((await provider.getRunStatus(a.run_id)).state,'COMPLETE');
await provider.cancelRun(b.run_id);assert.equal((await provider.getRunStatus(b.run_id)).state,'CANCELING');
await control({action:'advance'});assert.equal((await provider.getRunStatus(b.run_id)).state,'CANCELED');
assert.equal((await provider.getRunLog(a.run_id)).run_id,a.run_id);
await control({action:'fault',operation:'wes.create',name:'lost-response'});
await assert.rejects(provider.runWorkflow(req()));
await control({action:'fault',operation:'wes.list',name:'503'});
await assert.rejects(provider.listRuns());await provider.listRuns();
await control({action:'fault',operation:'wes.list',name:'malformed-json'});
await assert.rejects(provider.listRuns());
const replay=await(await fetch(`${base}/control/v1/sessions/${s.session_id}/replay`,{headers})).json();
assert.equal(replay.accepted_runs,3);
const evidence={client:'RestWesProvider',upstream_revision:'44c6f96fbfbeff6892ba0ca3836a7bcbd7c9069a',evidence_class:'simulated-protocol',runtime:'Node native fetch/FormData/File (browser API surface)',scenarios:['multipart','opaque-paging','cancel-completion-race','cancel-pending','lost-response','bounded-overload','malformed-json'],replay};
fs.writeFileSync(process.env.GA4GH_SCENARIO_BROWSER_EVIDENCE,JSON.stringify(evidence,null,2)+'\n');
console.log('RestWesProvider: seven shared WES scenarios passed; accepted runs = 3');
