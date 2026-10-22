"""Inventory owned UI text, excluding command input, paths, IDs and test fixtures.

Diagnostics/source data and exported JSON retain their original contents. The
manifest is committed so CI can require every supported catalog to cover it.
"""
import json,re
from pathlib import Path
ROOT=Path(__file__).resolve().parent.parent
MODULES=['app','dashboard','tool_window','widgets','a11y','theme','system_info','devcheck_ui','devinspect_ui','container_readiness_ui','build_log_ui','build_run_ui','dependency_ui','native_toolchain_ui','diagnostic_bundle_ui','envprofile_ui','projectdoctor_ui','project_readiness_ui']
TOKEN=re.compile(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"',re.S)
def literals(source):
    source=source.split('#[cfg(test)]')[0]
    for m in TOKEN.finditer(source):
        if not m[0].startswith('"'):continue
        try:value=json.loads(m[0])
        except json.JSONDecodeError:continue
        yield value,source[max(0,m.start()-100):m.start()]
def human(s):
    return len(s)>1 and re.search('[a-zA-Z]',s) and (' ' in s or s[0].isupper()) and not re.search(r'https?://|^[A-Za-z]:[\\/]|^\\\\|^[A-Z_]+$|^[/.-]',s) and not ('\n--' in s or s.startswith('--') or s.startswith('/p:'))
def canonical(s):
    n=0
    def replace(m):
        nonlocal n
        if m[0] in ('{{','}}'):return m[0]
        value='{p'+str(n)+'}';n+=1;return value
    return re.sub(r'\{\{|\}\}|\{[^{}]*\}',replace,s)
def collect():
    keys=set();templates=set()
    for name in MODULES:
        source=(ROOT/'src/gui'/f'{name}.rs').read_text(encoding='utf-8')
        for text,context in literals(source):
            if not human(text) and not re.search(r"\b(?:t|tf)\(\s*$",context):continue
            if re.search(r'format!\s*\(\s*$',context):
                # Formatting of raw data with no English words is deliberately excluded.
                if len(re.findall('[A-Za-z]',re.sub(r'\{[^{}]*\}','',text)))<4:continue
                templates.add(text);keys.add(canonical(text))
            else:keys.add(text)
    # Toolchain guidance/status metadata is displayed as owned UI prose.
    for module in ['devcheck.rs','devversion.rs','project_readiness.rs']:
        source=(ROOT/'src'/module).read_text(encoding='utf-8')
        for text,context in literals(source):
            if human(text) and not re.search(r'format!\s*\(\s*$',context):keys.add(text)
    # Scope warnings are UI guidance even when shared with technical reports.
    source=(ROOT/'src/container_readiness.rs').read_text(encoding='utf-8')
    for text,context in literals(source):
        if 'pub const LIMITATIONS:' in context:keys.add(text)
    # Action captions are distinct from stable enum identifiers.
    source=(ROOT/'src/dependencies/native.rs').read_text(encoding='utf-8').split('pub struct Options')[0]
    for text,context in literals(source):
        if human(text):keys.add(text)
    keys.discard('Some tool text is still in English. Missing translations use English.')
    # Exact technical names stay invariant, rather than misleading translated tool names.
    verbatim=sorted(k for k in keys if not re.search('[a-zA-Z]',k) or k in {'SetupScope','MSVC','CTest','MSBuild','CMake','CMake / Ninja','MSVC x64/x86','Windows SDK 10.0.26100','VIRTUAL_ENV','UTF-8','JSON','PATH','PATHEXT','TCP','UDP'} or re.fullmatch(r'(?:\.NET|Rust|CMake|Microsoft|Windows|SDK|MSVC|Ninja|WSL|ARM64|x64|x86|SDK|Runtime|Desktop|Build Tools|vs_buildtools\.exe| / | |[0-9.])+',k))
    templates.update(['Host {} / App {}','{} CPU Threads'])
    keys.update(canonical(t) for t in templates)
    verbatim=sorted(set(verbatim)|({'Python','Node.js','Rust / MSVC','CMake / C++','.NET','.NET SDK','.NET Desktop Runtime','Microsoft vs_buildtools.exe','ARM64'} & keys))
    stable={'CurrentBuildNumber','FileDescription','Hostarm64','Hostx64','MSBuild.dll','Microsoft Visual Studio','OriginalFilename','ProductName','ProgramFiles','ProgramFiles(x86)','SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion','SetupScope {p0}.0','SetupScope-diagnostics.json','SetupScope-offline-{p0}','VC/Tools/MSVC','VCToolsInstallDir','Windows Kits/10','WindowsSdkDir','{p0} — SetupScope','Title','review shown','review exists','action requires report','candidate files','current evidence'}
    verbatim=sorted((set(verbatim)|{k for k in keys if k in stable or re.fullmatch(r'(?:COM|LPT)[1-9]',k) or re.match(r'^[A-Z]:\\',k)})-{'All files'})
    return {'scope':'Owned text in the compiled desktop interface, including tooltips, safety guidance and formatted UI statuses. Original diagnostic evidence, captured logs, vendor output, command identifiers/arguments, paths, errors from dependencies/OS, JSON schemas and CLI help retain their original form.','keys':sorted(keys),'verbatim':verbatim},sorted(templates)
if __name__=='__main__':
    manifest,templates=collect()
    (ROOT/'lang/ui-strings.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    (ROOT/'lang/ui-templates.json').write_text(json.dumps(templates,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(f'{len(manifest["keys"])} owned UI keys; {len(templates)} formatted templates; {len(manifest["verbatim"])} technical invariants')
