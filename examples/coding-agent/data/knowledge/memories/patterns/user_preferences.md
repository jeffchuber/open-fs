# User Preferences

## Coding Style
- Prefers type hints on all functions
- Likes dataclasses over plain classes
- Prefers f-strings over .format()
- Uses black for formatting (line length 88)
- Prefers explicit imports over star imports

## Testing
- Uses pytest (not unittest)
- Prefers fixtures over setup/teardown
- Likes parametrized tests for edge cases
- Wants tests in separate test_*.py files

## Architecture
- Prefers dependency injection
- Likes separation of concerns
- Prefers composition over inheritance
- Uses repository pattern for data access

## Documentation
- Wants docstrings on public functions
- Prefers Google-style docstrings
- Likes inline comments for complex logic
- Wants README with setup instructions

## Security
- Always use bcrypt for passwords
- Prefers short-lived access tokens
- Wants rate limiting on auth endpoints
- Environment variables for secrets

## Communication
- Prefers concise explanations
- Likes seeing the plan before implementation
- Wants to review critical changes
- Appreciates explanations of trade-offs
