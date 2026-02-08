# Current Task

Implement JWT authentication for the FastAPI backend with the following requirements:

1. User registration with email/password
2. Login endpoint returning access + refresh tokens
3. Token refresh endpoint
4. Protected route middleware
5. Password hashing with bcrypt

## Acceptance Criteria

- [ ] POST /auth/register creates new user
- [ ] POST /auth/login returns JWT tokens
- [ ] POST /auth/refresh rotates tokens
- [ ] GET /users/me returns current user (protected)
- [ ] All passwords stored with bcrypt
- [ ] Tokens expire appropriately (access: 15min, refresh: 7days)
