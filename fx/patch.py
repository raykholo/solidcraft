import sys, struct
src, out = sys.argv[1], sys.argv[2]
s = open(src).read().replace("\r\n", "\n")
def esc(b): return "".join("\\%02x" % x for x in b)
def u32(*v): return struct.pack("<" + "I" * len(v), *v)
old = '(data (i32.const 3000) "\\01\\00\\00\\00\\1c\\0c\\00\\00\\01\\00\\00\\00\\00\\00\\00\\00\\00\\00\\00\\00")'
assert old in s
new_decl = '(data (i32.const 3000) "' + esc(u32(1, 3100, 1, 7000, 1)) + '")'
datas = []
def put(at, b): datas.append('(data (i32.const %d) "%s")' % (at, esc(b)))
def text(at, t): put(at, t.encode())
text(7300, "jobs"); text(7310, "Jobs"); text(7320, "ws_design"); text(7340, "Operations")
text(7360, "add"); text(7370, "clear"); text(7380, "refresh"); text(7400, "browser")
put(7100, b"\x00")
put(7112, u32(7310, 4, 7120, 1))
put(7120, u32(7340, 10, 7140, 3))
put(7140, u32(7360, 3, 7370, 5, 7380, 7))
put(7172, b"\x02\x00\x00\x00" + u32(7192, 1))
put(7192, u32(7300, 4))
put(7200, u32(7400, 7))
ws = u32(7300, 4, 7310, 4, 7320, 9, 7100, 1, 7112, 1, 7172, 1, 7200, 1)
ws += b"\x01\x00\x00\x00" + u32(7380, 7) + b"\x01\x00\x00\x00" + u32(7380, 7)
assert len(ws) == 80
put(7000, ws)
s = s.replace(old, new_decl + "\n    " + "\n    ".join(datas))
s = s.replace("__PACKAGE__", "craft:plugin").replace("__VERSION__", "0.2.0").replace("__COMMAND__", "inspect.measure").replace("__COMMAND_LEN__", "15").replace("__ARGS__", "{}").replace("__ARGS_LEN__", "2")
open(out, "w").write(s)
