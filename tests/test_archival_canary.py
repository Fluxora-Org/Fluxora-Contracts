import subprocess
from pathlib import Path

def test_archival_canary_dry_run():
    """Ensure the archival canary script is structurally valid bash."""
    script_path = Path("script/archival-canary.sh")
    assert script_path.exists(), f"{script_path} not found"
    result = subprocess.run(["bash", "-n", str(script_path)], capture_output=True, text=True)
    assert result.returncode == 0, f"Bash syntax check failed: {result.stderr}"
