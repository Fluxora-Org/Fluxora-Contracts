#!/usr/bin/env python3
import re
import sys

filepath = sys.argv[1]

with open(filepath, 'r') as f:
    content = f.read()

def count_args(call_content):
    inner = call_content[:-1].strip()
    if inner.endswith(','):
        inner = inner[:-1]
    if not inner:
        return 0
    arg_depth = 0
    arg_count = 1
    for ch in inner:
        if ch in '([{':
            arg_depth += 1
        elif ch in ')]}':
            arg_depth -= 1
        elif ch == ',' and arg_depth == 0:
            arg_count += 1
    return arg_count

def fix_calls(text):
    result = []
    i = 0
    fixes = 0
    while i < len(text):
        m = re.search(r'\.(try_)?create_stream\(', text[i:])
        if m is None:
            result.append(text[i:])
            break
        start = i + m.start()
        result.append(text[i:start + len(m.group())])
        i = start + len(m.group())
        depth = 1
        j = i
        while j < len(text) and depth > 0:
            if text[j] == '(':
                depth += 1
            elif text[j] == ')':
                depth -= 1
            j += 1
        call_content = text[i:j]
        nargs = count_args(call_content)
        if nargs == 7:
            inner_content = call_content[:-1]
            stripped = inner_content.rstrip()
            lines = stripped.split('\n')
            last_line = lines[-1]
            indent = len(last_line) - len(last_line.lstrip())
            indent_str = ' ' * indent
            if '\n' in stripped:
                new_call = stripped + '\n' + indent_str + '&0u32,\n' + indent_str + ')'
            else:
                new_call = stripped + ' &0u32,)'
            result.append(new_call)
            fixes += 1
        else:
            result.append(call_content)
        i = j
    return ''.join(result), fixes

new_content, fixes = fix_calls(content)
print(f"Fixed {fixes} calls in {filepath}")
with open(filepath, 'w') as f:
    f.write(new_content)
print("Done")
