import json, sys
d = json.load(open(sys.argv[1])); f = sys.argv[2]
for b in d.get('bodies', []):
    print('RESULT', f, 'faces', b.get('faces'), 'closed', b.get('closed'), 'volume', b.get('volume_mm3'), 'area', b.get('area_mm2'), 'types', b.get('face_types'))
for t in d.get('timeline', []):
    if t.get('warning'):
        print('WARNING', f, t['warning'])
