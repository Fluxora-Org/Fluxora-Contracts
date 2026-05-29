#!/usr/bin/env python3
import re
import sys

def fix_duplicate_fee(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
    
    # Fix pattern: &1000, &0u32, &0u32) -> &1000, &0u32)
    pattern1 = r', &0u32, &0u32\)'
    replacement1 = ', &0u32)'
    content = re.sub(pattern1, replacement1, content)
    
    # Fix pattern: &100&0u32, &0u32, -> &100, &0u32,
    pattern2 = r'&(\d+)(\s*)&0u32,(\s*)&0u32,'
    replacement2 = r'&\1,\2&0u32,'
    content = re.sub(pattern2, replacement2, content)
    
    # Fix pattern: &100&0u32, &0u32) -> &100, &0u32)
    pattern3 = r'&(\d+)(\s*)&0u32,(\s*)&0u32\)'
    replacement3 = r'&\1,\2&0u32)'
    content = re.sub(pattern3, replacement3, content)
    
    with open(filepath, 'w') as f:
        f.write(content)
    
    print(f"Fixed duplicates in {filepath}")

if __name__ == '__main__':
    files = ['src/test.rs', 'tests/integration_suite.rs', 'src/test_issue_39.rs']
    for filepath in files:
        fix_duplicate_fee(filepath)