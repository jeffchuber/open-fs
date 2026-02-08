# Debugging Skill

## Systematic Debugging Process

### 1. Reproduce the Issue
- Get exact steps to reproduce
- Identify minimal reproduction case
- Note environment details (OS, versions, etc.)

### 2. Gather Information
- Read error messages carefully
- Check logs (application, system, database)
- Review recent changes (git diff, git log)
- Check related issues/PRs

### 3. Form Hypotheses
- What could cause this behavior?
- List possible causes by likelihood
- Consider recent changes first

### 4. Test Hypotheses
- Add logging/print statements
- Use debugger breakpoints
- Write minimal test case
- Binary search through commits (git bisect)

### 5. Fix and Verify
- Make minimal fix
- Verify fix resolves issue
- Check for regressions
- Add test to prevent recurrence

## Common Debugging Commands

### Python
```python
# Interactive debugger
import pdb; pdb.set_trace()

# Or with IPython
import ipdb; ipdb.set_trace()

# Breakpoint (Python 3.7+)
breakpoint()

# Logging
import logging
logging.basicConfig(level=logging.DEBUG)
logger = logging.getLogger(__name__)
logger.debug(f"Variable state: {var}")
```

### Rust
```rust
// Debug print
dbg!(&variable);

// Pretty print
println!("{:#?}", variable);

// Backtrace
RUST_BACKTRACE=1 cargo run
```

### Git
```bash
# Find commit that introduced bug
git bisect start
git bisect bad HEAD
git bisect good v1.0.0

# See what changed
git diff HEAD~5..HEAD -- path/to/file.py

# Blame specific lines
git blame -L 50,60 file.py
```

## Error Pattern Recognition

| Error Type | Common Causes |
|------------|---------------|
| NullPointerException | Uninitialized variable, missing null check |
| IndexOutOfBounds | Off-by-one, empty collection |
| TimeoutError | Network issues, deadlock, infinite loop |
| MemoryError | Memory leak, large data structure |
| ImportError | Missing dependency, wrong path |
