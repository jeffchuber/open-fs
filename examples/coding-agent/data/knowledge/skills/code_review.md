# Code Review Skill

## Overview
Systematic approach to reviewing code for quality, correctness, and maintainability.

## Checklist

### 1. Correctness
- Does the code do what it's supposed to do?
- Are edge cases handled?
- Are there any off-by-one errors?
- Is error handling comprehensive?

### 2. Security
- Input validation present?
- SQL injection prevention?
- XSS protection?
- Sensitive data exposure?
- Authentication/authorization correct?

### 3. Performance
- Any N+1 queries?
- Unnecessary loops or iterations?
- Memory leaks possible?
- Caching opportunities?

### 4. Readability
- Clear variable/function names?
- Appropriate comments?
- Consistent formatting?
- Single responsibility principle?

### 5. Testing
- Unit tests present?
- Edge cases tested?
- Mocks used appropriately?
- Test coverage adequate?

## Response Format

```markdown
## Code Review: [filename]

### Summary
[1-2 sentence overview]

### Issues Found

#### Critical
- [Issue with line number and fix]

#### Warnings
- [Potential problems]

#### Suggestions
- [Nice-to-have improvements]

### Approved: [Yes/No/With Changes]
```
