"""Audit owned desktop text; original evidence and CLI output are outside scope."""
import collections
import json
import re
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parent.parent
LOCALES = ('de','es','fr','it','ja','ko','pl','pt-BR','ru','zh-CN')
FIELDS = re.compile(r'(?<!\{)\{[^{}]*\}(?!\})')
def unique_object(pairs):
    result = {}
    for key,value in pairs:
        if key in result: raise ValueError(f'duplicate JSON key: {key}')
        result[key] = value
    return result
def load(path):
    return json.loads(path.read_text(encoding='utf-8-sig'),object_pairs_hook=unique_object)
def main():
    manifest=load(ROOT/'lang/ui-strings.json')
    keys=manifest['keys']; invariants=set(manifest['verbatim'])
    assert len(keys)==len(set(keys)) and invariants<=set(keys)
    result={'scope':manifest['scope'],'owned_ui_keys':len(keys),'translatable_keys':len(keys)-len(invariants),'technical_invariants':len(invariants),'limits':'Coverage and placeholder integrity do not certify translation semantics, native-speaker review or rendered layout. Equal-source entries require human review.','languages':{}}
    failed=False
    for locale in LOCALES:
        data=load(ROOT/f'lang/{locale}.json')
        missing=[key for key in keys if not isinstance(data.get(key),str) or not data[key].strip()]
        fields=[key for key in keys if key not in missing and collections.Counter(FIELDS.findall(key))!=collections.Counter(FIELDS.findall(data[key]))]
        changed=sorted(key for key in invariants if key not in missing and data[key]!=key)
        same=[key for key in keys if key not in invariants and data.get(key)==key]
        result['languages'][locale]={'covered_ui_keys':len(keys)-len(missing),'missing_keys':missing,'placeholder_errors':fields,'changed_technical_invariants':changed,'equal_source_review':same}
        failed|=bool(missing or fields or changed)
        print(f'{locale}: {len(keys)-len(missing)}/{len(keys)}, placeholders={len(fields)}, invariants={len(changed)}, equal-source review={len(same)}')
    (ROOT/'docs/LOCALIZATION-COVERAGE.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    return int(failed)
if __name__=='__main__': sys.exit(main())
