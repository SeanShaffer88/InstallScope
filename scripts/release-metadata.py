"""Generate a scoped CycloneDX inventory and preserve dependency license notices."""
from pathlib import Path
import datetime, hashlib, json, re, shutil, subprocess, sys

repo=Path(__file__).resolve().parent.parent
destination=Path(sys.argv[1]).resolve()
destination.mkdir(parents=True,exist_ok=True)
metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--offline','--format-version','1','--filter-platform','x86_64-pc-windows-msvc'],cwd=repo))
nodes={n['id']:n for n in metadata['resolve']['nodes']}
packages={p['id']:p for p in metadata['packages'] if p['id'] in nodes}
refs={key:'cargo-'+hashlib.sha256((p['name']+'@'+p['version']+'|'+str(p.get('source'))).encode()).hexdigest()[:24] for key,p in packages.items()}
components=[];notices=[];missing=[]
for key,p in sorted(packages.items(),key=lambda x:(x[1]['name'],x[1]['version'])):
    component={'type':'application' if p['name']=='setupscope' else 'library','bom-ref':refs[key],'name':p['name'],'version':p['version'],'purl':f"pkg:cargo/{p['name']}@{p['version']}"}
    if p.get('license'):component['licenses']=[{'expression':p['license']}]
    if p.get('repository') and p['repository'].startswith(('https://','http://')):component['externalReferences']=[{'type':'vcs','url':p['repository']}]
    components.append(component)
    root=Path(p['manifest_path']).parent
    copied=[]
    # Source checkout is covered by the root project license, not recursively scanned.
    if p['name']!='setupscope':
        for file in root.rglob('*'):
            if not file.is_file() or file.is_symlink() or file.stat().st_size>1024*1024:continue
            relative=file.relative_to(root)
            if any(x in {'target','.git','tests','benches'} for x in relative.parts):continue
            if not re.search(r'(?i)(^|[-_.])(licen[cs]e|copying|notice|copyright|authors)([-_.]|$)',file.name) and not ('fonts' in relative.parts and file.suffix.lower()=='.txt'):continue
            try:file.read_text(encoding='utf-8')
            except (UnicodeError,OSError):continue
            target=destination/'licenses'/(p['name']+'-'+p['version'])/relative
            target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(file,target);copied.append(str(relative).replace('\\','/'))
        supplemental=repo/'docs/licenses/crates'/(p['name']+'-'+p['version'])
        if supplemental.exists():
            for file in supplemental.iterdir():
                if file.is_file():
                    target=destination/'licenses'/(p['name']+'-'+p['version'])/file.name
                    target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(file,target);copied.append(file.name)
        if not copied:missing.append(p['name']+' '+p['version'])
    notices.append({'name':p['name'],'version':p['version'],'declared_license':p.get('license'),'notices':copied,'repository':p.get('repository')})
document={'bomFormat':'CycloneDX','specVersion':'1.6','version':1,'metadata':{'timestamp':datetime.datetime.now(datetime.timezone.utc).isoformat(),'properties':[{'name':'setupscope:scope','value':'Cargo resolved Windows x64 dependency inventory, including build/dev graph; not a binary reachability or vulnerability certification.'}]},'components':components,'dependencies':[{'ref':refs[key],'dependsOn':[refs[x] for x in node['dependencies'] if x in refs]} for key,node in nodes.items() if key in refs]}
(destination/'sbom.cdx.json').write_text(json.dumps(document,indent=2),encoding='utf-8')
(destination/'THIRD-PARTY-NOTICES.json').write_text(json.dumps({'scope':'Cargo resolved Windows x64 graph; upstream declarations and collected notice files','packages':notices,'notice_files_unavailable':missing},indent=2),encoding='utf-8')
print(f'Inventory: {len(components)} components; notice text unavailable for {len(missing)} packages.')
if missing:print('Declared-license-only entries: '+', '.join(missing))
