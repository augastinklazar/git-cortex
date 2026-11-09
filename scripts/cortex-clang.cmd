@echo off
python -c "import sys, subprocess; args = ['clang.exe'] + [('--target=x86_64-w64-windows-gnu' if a.startswith('--target=') else a) for a in sys.argv[1:]]; sys.exit(subprocess.run(args).returncode)" %*
