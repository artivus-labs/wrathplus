#!/usr/bin/env python3
"""Opt-in ratchets. No hook installation, baseline rewriting, or tree formatting."""
import argparse
import collections
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent


def run(args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def tracked():
    return [p for p in subprocess.check_output(['git', 'ls-files', '-z']).decode().split('\0') if p]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load(name):
    return json.loads((HERE / (name + '-baseline.json')).read_text())


# Generated sources, exempt from the line limit by name (their generator is the thing to keep
# small). A move of one of these needs this list updated in the same commit.
GENERATED = {
    'crates/wrathplus-protocol/src/messages/update_object/fields/layout_generated.rs',
}


def loc(paths):
    baseline = load('loc')['files']
    failures = []
    for path in paths:
        if path in GENERATED:
            continue
        if path.endswith('.rs') and Path(path).is_file():
            count = len(Path(path).read_bytes().splitlines())
            if count > max(500, baseline.get(path, 500)):
                failures.append(f'{path}: {count} > {max(500, baseline.get(path, 500))}')
    if failures:
        raise ValueError('\n'.join(failures))


# WS2's gate (https://github.com/artivus-labs/wrathplus/wiki/WRATH-NATIVE-PLAN): production code in these crates names descriptor fields
# through `Field`, never a raw index. The descriptor names come off the generated layout.
WS2_ROOTS = ('crates/wrathplus-app/src/', 'crates/wrathplus-app-core/src/', 'crates/wrathplus-app-net/src/',
             'crates/wrathplus-app-world/src/', 'crates/wrathplus-app-ui/src/', 'crates/wrathplus-app-tools/src/',
             'crates/wrathplus-world/src/')
WS2_LAYOUT = 'crates/wrathplus-protocol/src/types/update_object/fields/layout_generated.rs'
WS2_TYPES = ('CONTAINER', 'CORPSE', 'DYNAMICOBJECT', 'GAMEOBJECT', 'ITEM', 'OBJECT', 'PLAYER', 'UNIT')


def ws2_names():
    """Every `Field` variant as the 1.12 constants spelled it, `UNIT_HEALTH` and `UNIT_FIELD_HEALTH`."""
    text = Path(WS2_LAYOUT).read_text()
    body = text.split('pub enum Field {', 1)[1].split('}', 1)[0]
    names = set()
    for variant in re.findall(r'^\s*([A-Z][A-Za-z0-9]*),', body, re.M):
        snake = re.sub(r'(?<!^)(?=[A-Z])', '_', variant).upper()
        names.add(snake)
        head, _, tail = snake.partition('_')
        if head in WS2_TYPES and tail:
            names.add(f'{head}_FIELD_{tail}')
    return names


def ws2_production(path, text):
    """The file's production code: test files skipped, a file cut at its first test module."""
    leaf = path.rsplit('/', 1)[-1]
    if '/tests/' in path or '_tests/' in path or 'test' in leaf or 'fixture' in leaf:
        return ''
    cut = re.search(r'^#\[cfg\(test\)\]\s*\n\s*(?:#\[path[^\n]*\n\s*)?(?:pub(?:\([^)]*\))? )?mod ', text, re.M)
    return text[:cut.start()] if cut else text


def ws2(paths):
    names = ws2_names()
    const = re.compile(r'\bconst ([A-Z][A-Z0-9_]*): u16 = (?:0x[0-9a-fA-F_]+|[0-9][0-9_]*);')
    failures = []
    for path in paths:
        if not path.endswith('.rs') or not path.startswith(WS2_ROOTS) or not Path(path).is_file():
            continue
        code = ws2_production(path, Path(path).read_text())
        for number, line in enumerate(code.splitlines(), 1):
            if line.lstrip().startswith('//'):
                continue
            for name in const.findall(line):
                if any(name == n or name.endswith('_' + n) for n in names):
                    failures.append(f'{path}:{number}: raw descriptor const {name}')
            if re.search(r'\bFIELD_[A-Z0-9_]+\b', line):
                failures.append(f'{path}:{number}: FIELD_* name; read the `Field` instead')
            if re.search(r'from_pairs\(&\[\s*\(\s*(?:0x[0-9a-fA-F_]+|[0-9][0-9_]*)\s*,', line):
                failures.append(f'{path}:{number}: literal from_pairs key; use `Field::X.index()`')
    if failures:
        raise ValueError('\n'.join(failures))


def fmt(paths):
    # skip_children prevents rustfmt following module declarations into unstaged files.
    for path in paths:
        if path.endswith('.rs') and Path(path).is_file():
            run(['rustfmt', '--check', '--edition', '2021', '--config', 'skip_children=true', path])


def crap(report):
    """Any modified file loses its legacy exceptions (stricter than per-function change)."""
    baseline = load('crap')
    entries = json.loads(Path(report).read_text())['entries']
    if not entries:
        raise ValueError('empty CRAP report is not coverage evidence')
    failures = []
    seen = set()
    for entry in entries:
        path = os.path.relpath(entry['file'])
        if not Path(path).is_file():
            raise ValueError(f'CRAP source not found: {path}')
        identity = (path, entry['function'], entry['line'])
        if identity in seen:
            continue  # cargo-crap can discover the same file via multiple workspace roots
        seen.add(identity)
        old = baseline['files'].get(path)
        unchanged = old and old['sha256'] == digest(path)
        # Multiset of scores avoids silently accepting additional same-named functions.
        key = entry['function']
        allowance = old['functions'].get(key, []) if unchanged else []
        if entry['crap'] > 10:
            candidates = [score for score in allowance if entry['crap'] <= score + 0.000001]
            if candidates:
                allowance.remove(min(candidates))
            else:
                failures.append(f"{path}:{entry['line']} {key}: CRAP {entry['crap']} > 10")
    if failures:
        raise ValueError('\n'.join(failures))


def clippy(report):
    baseline = load('clippy')['diagnostics']
    remaining = collections.Counter(baseline)
    failures = []
    for line in Path(report).read_text().splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if item.get('reason') != 'compiler-message':
            continue
        message = item['message']
        if message['level'] not in ('warning', 'error'):
            continue
        spans = [s for s in message['spans'] if s['is_primary']]
        # Cargo's -D warnings promotes warnings to errors. Known warnings alone are allowed;
        # unrelated compiler errors always fail, even when a baseline matches.
        code = (message.get('code') or {}).get('code', '')
        if not spans or not code:
            failures.append(message.get('rendered') or message['message'])
            continue
        span = spans[0]
        path = span['file_name']
        key = json.dumps([path, code, message['message'], digest(path)], separators=(',', ':'))
        if remaining[key]:
            remaining[key] -= 1
        else:
            failures.append(message.get('rendered') or key)
    if failures:
        raise ValueError('\n'.join(failures))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('check', choices=['loc', 'fmt', 'ws2', 'crap-report', 'clippy-report', 'fast'])
    parser.add_argument('paths', nargs='*')
    args = parser.parse_args()
    if args.check.endswith('-report'):
        if len(args.paths) != 1:
            parser.error('exactly one report required')
        (crap if args.check == 'crap-report' else clippy)(args.paths[0])
    else:
        paths = args.paths or tracked()
        if args.check in ('loc', 'fast'):
            loc(paths)
        if args.check in ('fmt', 'fast'):
            fmt(paths)
        if args.check == 'ws2':
            ws2(paths)
    print(f'quality: {args.check} passed')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, subprocess.CalledProcessError) as error:
        print(f'quality: {error}', file=sys.stderr)
        sys.exit(1)
