import re, sys, glob, json
from janome.tokenizer import Tokenizer
TOK = Tokenizer()

JA = r'぀-ゟ゠-ヿ一-鿿々ー'   # kana, kanji, iteration/長音
JA_RUN = re.compile(f'[{JA}][{JA}\\u3001\\u3002\\uFF08\\uFF09\\u300C\\u300D\\uFF1A\\uFF1F\\uFF01 ]*')
LATIN  = re.compile(r"[A-Za-z][A-Za-z0-9'’_\-]*")
SKIP_POS = ("記号", "補助記号", "空白")

def ja_words(text):
    runs = [m.group(0) for m in JA_RUN.finditer(text)]
    if not runs: return 0, 0
    chars = sum(len(re.sub(f'[^{JA}]', '', r)) for r in runs)
    n = 0
    for r in runs:
        for t in TOK.tokenize(r):
            if t.surface.strip() and not t.part_of_speech.split(',')[0] in SKIP_POS:
                n += 1
    return n, chars

def latin_words(text):
    return len(LATIN.findall(text))

def strip_code_fences(md):
    return re.sub(r'```.*?```', ' ', md, flags=re.S)

def split_rust(src):
    """return (comment_text, code_text)"""
    comments, code = [], []
    i, n = 0, len(src)
    while i < n:
        if src.startswith('//', i):
            j = src.find('\n', i); j = n if j < 0 else j
            comments.append(src[i:j]); i = j
        elif src.startswith('/*', i):
            j = src.find('*/', i+2); j = n if j < 0 else j+2
            comments.append(src[i:j]); i = j
        elif src[i] == '"':
            j = i+1
            while j < n and src[j] != '"':
                j += 2 if src[j] == '\\' else 1
            code.append(src[i:j+1]); i = j+1
        else:
            code.append(src[i]); i += 1
    return ''.join(comments), ''.join(code)

def test_spans(src):
    """spans of #[cfg(test)] mod blocks (brace-matched, comment/string-aware)"""
    spans = []
    for m in re.finditer(r'#\[cfg\(test\)\]', src):
        i, n = m.end(), len(src)
        while i < n and src[i] != '{':          # skip to the mod's opening brace
            if src.startswith('//', i):
                j = src.find('\n', i); i = n if j < 0 else j
            i += 1
        depth, j = 0, i
        while j < n:
            c = src[j]
            if src.startswith('//', j):
                k = src.find('\n', j); j = n if k < 0 else k
            elif src.startswith('/*', j):
                k = src.find('*/', j+2); j = n if k < 0 else k+2
            elif c == '"':
                j += 1
                while j < n and src[j] != '"':
                    j += 2 if src[j] == '\\' else 1
                j += 1
            else:
                if c == '{': depth += 1
                elif c == '}':
                    depth -= 1
                    if depth == 0: j += 1; break
                j += 1
        spans.append((m.start(), j))
    return spans

def report(label, files, kind):
    jw = jc = lw = 0
    for f in files:
        t = open(f, encoding='utf-8', errors='replace').read()
        if kind == 'md':   t = strip_code_fences(t)
        a, b = ja_words(t); jw += a; jc += b; lw += latin_words(t)
    return dict(label=label, files=len(files), ja_words=jw, ja_chars=jc,
                latin_words=lw, total=jw+lw)

root = '/Users/aogaki/Workspace/delila-rs/'
out = []
out.append(report('TODO/ task documents', sorted(glob.glob(root+'TODO/**/*.md', recursive=True)), 'md'))
out.append(report('docs/ design documents', sorted(glob.glob(root+'docs/**/*.md', recursive=True)), 'md'))

# rust: comments vs code, code further split production vs test
# (test = #[cfg(test)] mod blocks in src/ + everything in tests/)
rs = sorted(glob.glob(root+'src/**/*.rs', recursive=True))
it = sorted(glob.glob(root+'tests/*.rs'))
cj = cc = cl = 0
prod_tok = test_tok = 0
n_testfn = 0
KW = re.compile(r"[A-Za-z_][A-Za-z0-9_]*|\b\d+\b")
TESTFN = re.compile(r'#\[(?:tokio::)?test[\](]')
for f in rs + it:
    s = open(f, encoding='utf-8', errors='replace').read()
    com, _ = split_rust(s)
    a, b = ja_words(com); cj += a; cc += b; cl += latin_words(com)
    n_testfn += len(TESTFN.findall(s))
    if f in it:
        prod_s, test_s = '', s
    else:
        spans = test_spans(s)
        test_s = ''.join(s[a:b] for a, b in spans)
        keep, pos = [], 0
        for a, b in spans:
            keep.append(s[pos:a]); pos = b
        keep.append(s[pos:])
        prod_s = ''.join(keep)
    _, cod = split_rust(prod_s); prod_tok += len(KW.findall(cod))
    _, cod = split_rust(test_s); test_tok += len(KW.findall(cod))
out.append(dict(label='comments in src/ + tests/', files=len(rs)+len(it),
                ja_words=cj, ja_chars=cc, latin_words=cl, total=cj+cl))
out.append(dict(label='Rust production code tokens', files=len(rs), ja_words=0,
                ja_chars=0, latin_words=prod_tok, total=prod_tok))
out.append(dict(label='Rust test code tokens', files=len(rs)+len(it), ja_words=0,
                ja_chars=0, latin_words=test_tok, total=test_tok,
                test_fns=n_testfn))
json.dump(out, open(sys.argv[1], 'w'), ensure_ascii=False, indent=1)
w = max(len(o['label']) for o in out)
print(f"{'':{w}}  {'files':>6} {'JA words':>10} {'(JA chars)':>11} {'EN words':>10} {'TOTAL':>11}")
for o in out:
    print(f"{o['label']:{w}}  {o['files']:6d} {o['ja_words']:10,} {o['ja_chars']:11,} "
          f"{o['latin_words']:10,} {o['total']:11,}")
