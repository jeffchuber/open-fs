# FastAPI Authentication Guide

## Overview

FastAPI provides built-in support for various authentication methods through
its security utilities in `fastapi.security`.

## OAuth2 with Password Flow

The most common authentication pattern for APIs.

### Setup

```python
from fastapi import Depends, FastAPI, HTTPException, status
from fastapi.security import OAuth2PasswordBearer, OAuth2PasswordRequestForm

app = FastAPI()
oauth2_scheme = OAuth2PasswordBearer(tokenUrl="token")
```

### Token Endpoint

```python
@app.post("/token")
async def login(form_data: OAuth2PasswordRequestForm = Depends()):
    user = authenticate_user(form_data.username, form_data.password)
    if not user:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Incorrect username or password",
            headers={"WWW-Authenticate": "Bearer"},
        )
    access_token = create_access_token(data={"sub": user.username})
    return {"access_token": access_token, "token_type": "bearer"}
```

### Protected Routes

```python
@app.get("/users/me")
async def read_users_me(token: str = Depends(oauth2_scheme)):
    user = get_user_from_token(token)
    return user
```

## JWT Tokens

JSON Web Tokens are the standard for stateless authentication.

### Structure

```
header.payload.signature

Header:  {"alg": "HS256", "typ": "JWT"}
Payload: {"sub": "user_id", "exp": 1234567890}
Signature: HMACSHA256(base64(header) + "." + base64(payload), secret)
```

### Best Practices

1. **Short expiration** - Access tokens should expire in 15-60 minutes
2. **Refresh tokens** - Use longer-lived refresh tokens to get new access tokens
3. **Secure storage** - Store tokens in httpOnly cookies or secure storage
4. **HTTPS only** - Never send tokens over unencrypted connections

## Dependencies

Required packages:
```
python-jose[cryptography]  # JWT encoding/decoding
passlib[bcrypt]           # Password hashing
```

## Security Considerations

- Never store plain-text passwords
- Use bcrypt or argon2 for password hashing
- Rotate secrets periodically
- Implement rate limiting on auth endpoints
- Log authentication failures
- Use secure random for token generation
