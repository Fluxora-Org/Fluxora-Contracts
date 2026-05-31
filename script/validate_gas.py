import re
import json
import subprocess
import sys
from typing import Dict, Any

def extract_baselines(file_path: str) -> Dict[str, Any]:
    with open(file_path, 'r') as f:
        content = f.read()
        match = re.search(r'<!-- GAS_BASELINE_START -->\s*(\{.*?\})\s*<!-- GAS_BASELINE_END -->', content, re.DOTALL)
        if not match:
            raise ValueError("Could not find gas baseline block in docs/gas.md")
        return json.loads(match.group(1))

def run_tests() -> str:
    print("Running gas regression tests...")
    result = subprocess.run(
        ["cargo", "test", "-p", "fluxora_stream", "--test", "gas_regression", "--", "--nocapture"],
        capture_output=True,
        text=True,
        env={"PATH": f"{subprocess.os.environ.get('HOME', '/Users/aditya')}/.cargo/bin:{subprocess.os.environ.get('PATH', '')}"}
    )
    # We ignore the return code here because the script handles the actual validation
    return result.stdout

def parse_measurements(output: str) -> Dict[str, Dict[str, int]]:
    measurements = {}
    # Pattern: GAS_MEASUREMENT: <function>: <size|single>: <cost>
    pattern = re.compile(r'GAS_MEASUREMENT: ([^:]+): ([^:]+): (\d+)')
    for line in output.splitlines():
        match = pattern.search(line)
        if match:
            func, size, cost = match.groups()
            if func not in measurements:
                measurements[func] = {}
            measurements[func][size] = int(cost)
    return measurements

def main():
    try:
        baselines = extract_baselines('docs/gas.md')
        output = run_tests()
        measured = parse_measurements(output)

        if not measured:
            print("Error: No gas measurements found in test output.")
            sys.exit(1)

        regressions = []
        print("\nGas Cost Report:")
        print(f"{'Function':<20} | {'Size':<10} | {'Baseline':<12} | {'Measured':<12} | {'Diff %':<10} | {'Status'}")
        print("-" * 80)

        for func, sizes in measured.items():
            for size, cost in sizes.items():
                # Get baseline value
                baseline_val = None
                if func == 'batch_withdraw':
                    baseline_val = baselines.get('batch_withdraw', {}).get(size)
                else:
                    # For non-batch functions, 'size' is 'single', we look for the function name key
                    baseline_val = baselines.get(func)

                if baseline_val is None:
                    print(f"{func:<20} | {size:<10} | {'N/A':<12} | {cost:<12} | {'N/A':<10} | MISSING")
                    continue

                diff = (cost - baseline_val) / baseline_val if baseline_val != 0 else 0
                status = "FAIL" if diff > 0.05 else "PASS"
                if status == "FAIL":
                    regressions.append((func, size, diff))

                print(f"{func:<20} | {size:<10} | {baseline_val:<12} | {cost:<12} | {diff:>8.2%} | {status}")

        if regressions:
            print("\nFAILED: Gas regression detected (> 5% increase) in the following functions:")
            for func, size, diff in regressions:
                print(f"- {func} ({size}): {diff:.2%}")
            sys.exit(1)
        else:
            print("\nSUCCESS: No gas regressions detected.")
            sys.exit(0)

    except Exception as e:
        print(f"Error during validation: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
