# Refactoring Skill

## Principles

1. **Make it work, make it right, make it fast** (in that order)
2. **Boy Scout Rule**: Leave code better than you found it
3. **Red-Green-Refactor**: Tests pass → Refactor → Tests still pass

## Common Refactoring Patterns

### Extract Method
**When**: Code block does one specific thing, or code is duplicated

```python
# Before
def process_order(order):
    # Validate order
    if not order.items:
        raise ValueError("Empty order")
    if not order.customer:
        raise ValueError("No customer")
    # ... more validation ...

    # Calculate total
    total = 0
    for item in order.items:
        total += item.price * item.quantity
    # ... rest of processing

# After
def process_order(order):
    validate_order(order)
    total = calculate_total(order)
    # ... rest of processing

def validate_order(order):
    if not order.items:
        raise ValueError("Empty order")
    if not order.customer:
        raise ValueError("No customer")

def calculate_total(order):
    return sum(item.price * item.quantity for item in order.items)
```

### Replace Conditional with Polymorphism
**When**: Switch/if-else on type, repeated type checking

```python
# Before
def calculate_area(shape):
    if shape.type == "circle":
        return 3.14 * shape.radius ** 2
    elif shape.type == "rectangle":
        return shape.width * shape.height
    elif shape.type == "triangle":
        return 0.5 * shape.base * shape.height

# After
class Circle:
    def area(self):
        return 3.14 * self.radius ** 2

class Rectangle:
    def area(self):
        return self.width * self.height

class Triangle:
    def area(self):
        return 0.5 * self.base * self.height
```

### Introduce Parameter Object
**When**: Multiple parameters that travel together

```python
# Before
def create_user(name, email, street, city, zip_code, country):
    ...

# After
@dataclass
class Address:
    street: str
    city: str
    zip_code: str
    country: str

def create_user(name: str, email: str, address: Address):
    ...
```

### Replace Magic Numbers with Constants
**When**: Literal values with unclear meaning

```python
# Before
if user.age >= 18:
    ...
if retry_count > 3:
    ...

# After
LEGAL_AGE = 18
MAX_RETRIES = 3

if user.age >= LEGAL_AGE:
    ...
if retry_count > MAX_RETRIES:
    ...
```

## Refactoring Safely

1. **Have tests first** - Never refactor without test coverage
2. **Small steps** - One change at a time, run tests between
3. **Version control** - Commit frequently, easy to revert
4. **No behavior change** - Refactoring doesn't change functionality
5. **Review the diff** - Ensure changes are intentional
