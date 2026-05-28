#!/bin/bash
# Find all create_stream calls with 7 arguments and add &0u32 as 8th

# Fix test.rs
echo "Fixing test.rs..."
sed -i 's/\.create_stream(\([^)]*\))/.create_stream(\1, \&0u32)/g' src/test.rs
sed -i 's/\.create_stream(\([^)]*\),)/.create_stream(\1, \&0u32,)/g' src/test.rs

# Fix integration_suite.rs
echo "Fixing integration_suite.rs..."
sed -i 's/\.create_stream(\([^)]*\))/.create_stream(\1, \&0u32)/g' tests/integration_suite.rs
sed -i 's/\.create_stream(\([^)]*\),)/.create_stream(\1, \&0u32,)/g' tests/integration_suite.rs

# Fix test_issue_39.rs
echo "Fixing test_issue_39.rs..."
sed -i 's/\.create_stream(\([^)]*\))/.create_stream(\1, \&0u32)/g' src/test_issue_39.rs
sed -i 's/\.create_stream(\([^)]*\),)/.create_stream(\1, \&0u32,)/g' src/test_issue_39.rs

echo "Done"