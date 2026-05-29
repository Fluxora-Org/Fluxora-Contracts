#!/usr/bin/env python3
import re
import sys

def fix_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
    
    # Pattern to match create_stream calls with 7 arguments
    # Look for .create_stream( ... ) with 7 args
    lines = content.split('\n')
    new_lines = []
    
    for line in lines:
        # Check if line has .create_stream( with potentially missing 8th arg
        if '.create_stream(' in line:
            # Count parentheses to find the end of the call
            # This is a simplified approach - for complex cases we'd need a parser
            pass
        
        new_lines.append(line)
    
    # Simple regex approach for now
    # Pattern: .create_stream( ... ) where ... doesn't have 8th arg
    # This is complex - better to use AST
    
    return '\n'.join(new_lines)

if __name__ == '__main__':
    if len(sys.argv) < 2:
        print("Usage: python3 fix_all_calls.py <file>")
        sys.exit(1)
    
    filepath = sys.argv[1]
    new_content = fix_file(filepath)
    
    with open(filepath, 'w') as f:
        f.write(new_content)
    
    print(f"Processed {filepath}")