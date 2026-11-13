#!/usr/bin/env python3
"""Reproduce bounded static observations, without loading/executing Windows code."""
from pathlib import Path
import hashlib, json, struct
root = Path(__file__).resolve().parents[1]
vendor = root/'vendor/Hummingbird-M-Update-Tool'
facts = {'source':'https://docs.aimachip.com/zh-cn/latest/_static/document/SU-03T/Hummingbird-M-Update-Tool.zip', 'files':{}}
for path in sorted(vendor.iterdir()):
    if path.is_file():
        b=path.read_bytes()
        facts['files'][path.name]={'size':len(b),'sha256':hashlib.sha256(b).hexdigest()}
b=(vendor/'UniCommunicateSwitch.dll').read_bytes()
# PE mapping: RVA -> raw file offset, validated against each section.
pe=struct.unpack_from('<I',b,0x3c)[0]
assert b[pe:pe+4] == b'PE\0\0'
count=struct.unpack_from('<H',b,pe+6)[0]
opt_size=struct.unpack_from('<H',b,pe+20)[0]
sections=[]
for i in range(count):
    pos=pe+24+opt_size+i*40
    size,rva,rawsize,raw=struct.unpack_from('<IIII',b,pos+8)
    sections.append((rva,rawsize,raw))
def offset(rva):
    for start,size,raw in sections:
        if start<=rva<start+size: return raw+rva-start
    raise ValueError(hex(rva))
start=offset(0x1f3a0)
table=struct.unpack_from('<256H',b,start)
reference=[]
for i in range(256):
    crc=i<<8
    for _ in range(8):crc=((crc<<1)^(0x1021 if crc&0x8000 else 0))&65535
    reference.append(crc)
assert list(table)==reference
facts['dll_crc_table']={'va':'0x1001f3a0','file_offset':hex(start),'polynomial':'0x1021','all_256_entries_match':True,'initial_value_from_assembly':0}
for kind in ['debug','release']:
    full=(vendor/f'uni_app_{kind}.bin').read_bytes()
    update=(vendor/f'uni_app_{kind}_update.bin').read_bytes()
    assert full[65536:]==update
facts['samples']={'update_equals_full_slice_from':'0x10000','not_a_general_format_validator':True}
output = root/'analysis/wire-facts.json'
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(facts,indent=2)+'\n')
print('Verified CRC table and both update/full image relations; wrote analysis/wire-facts.json')
