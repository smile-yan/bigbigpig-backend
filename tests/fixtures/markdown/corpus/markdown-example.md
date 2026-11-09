# Markdown 示例文档

本文档展示了博客支持的各类 Markdown 语法，包括代码高亮、数学公式等。

---

## 1. 标题

```markdown
# 一级标题
## 二级标题
### 三级标题
#### 四级标题
##### 五级标题
###### 六级标题
```

---

## 2. 文本格式

这是**粗体文本**，这是*斜体文本*，这是~~删除线~~，这是 `行内代码`。

**组合使用：**_**粗体加斜体**_，`行内代码中的**粗体**`

> 这是一个引用块
> 可以换行
> > 嵌套引用

---

## 3. 代码块

### Python

```python
import numpy as np
import pandas as pd
from typing import List, Dict, Optional

class DataProcessor:
    """数据处理器类"""

    def __init__(self, config: Dict[str, any]):
        self.config = config
        self.data = None

    def load_data(self, filepath: str) -> pd.DataFrame:
        """加载数据"""
        return pd.read_csv(filepath)

    def process(self, df: pd.DataFrame) -> np.ndarray:
        """处理数据"""
        return df.values.mean(axis=1)

    def predict(self, X: np.ndarray) -> List[float]:
        """预测"""
        return [self.model.predict(x) for x in X]
```

### Go

```go
package main

import (
    "fmt"
    "context"
    "time"
)

type Server struct {
    addr string
    port int
}

func NewServer(addr string, port int) *Server {
    return &Server{
        addr: addr,
        port: port,
    }
}

func (s *Server) Start(ctx context.Context) error {
    fmt.Printf("Server starting on %s:%d\n", s.addr, s.port)

    <-ctx.Done()
    return nil
}
```

### Java

```java
public class UserService {

    private final UserRepository userRepository;
    private final PasswordEncoder passwordEncoder;

    public UserService(UserRepository userRepository,
                       PasswordEncoder passwordEncoder) {
        this.userRepository = userRepository;
        this.passwordEncoder = passwordEncoder;
    }

    public User createUser(String username, String email, String password) {
        if (userRepository.existsByUsername(username)) {
            throw new UserAlreadyExistsException("Username already exists");
        }

        User user = new User();
        user.setUsername(username);
        user.setEmail(email);
        user.setPassword(passwordEncoder.encode(password));

        return userRepository.save(user);
    }

    public Optional<User> findById(Long id) {
        return userRepository.findById(id);
    }
}
```

### C / C++

```c
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    int id;
    char name[100];
    float score;
} Student;

void print_student(const Student *s) {
    printf("ID: %d, Name: %s, Score: %.2f\n",
           s->id, s->name, s->score);
}

int main() {
    Student s = {1, "Alice", 95.5};
    print_student(&s);
    return 0;
}
```

```cpp
#include <iostream>
#include <vector>
#include <algorithm>
#include <memory>

template<typename T>
class Container {
private:
    std::vector<T> data;

public:
    void add(const T& item) {
        data.push_back(item);
    }

    void sort() {
        std::sort(data.begin(), data.end());
    }

    size_t size() const {
        return data.size();
    }
};

int main() {
    auto container = std::make_unique<Container<int>>();
    container->add(5);
    container->add(2);
    container->add(8);
    container->sort();

    std::cout << "Size: " << container->size() << std::endl;
    return 0;
}
```

### HTML / CSS

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <title>示例页面</title>
    <style>
        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 20px;
        }
        .header {
            background: linear-gradient(to right, #667eea, #764ba2);
            color: white;
            padding: 30px;
        }
    </style>
</head>
<body>
    <div class="container">
        <header class="header">
            <h1>欢迎访问</h1>
        </header>
    </div>
</body>
</html>
```

```css
/* CSS 动画示例 */
@keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
}

.fade-enter-active {
    animation: fadeIn 0.5s ease-in-out;
}

.button {
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    border: none;
    padding: 12px 24px;
    color: white;
    border-radius: 8px;
    cursor: pointer;
    transition: transform 0.2s;
}

.button:hover {
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
}
```

### JavaScript

```javascript
// ES6+ 特性示例
const fetchData = async (url) => {
  try {
    const response = await fetch(url);
    const data = await response.json();
    return { success: true, data };
  } catch (error) {
    console.error('Error:', error);
    return { success: false, error: error.message };
  }
};

class EventEmitter {
  constructor() {
    this.events = new Map();
  }

  on(event, callback) {
    if (!this.events.has(event)) {
      this.events.set(event, []);
    }
    this.events.get(event).push(callback);
  }

  emit(event, ...args) {
    const callbacks = this.events.get(event) || [];
    callbacks.forEach(cb => cb(...args));
  }
}

export default EventEmitter;
```

### Bash / Shell

```bash
#!/bin/bash

# 变量定义
NAME="World"
CURRENT_DIR=$(pwd)
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# 函数定义
backup_files() {
    local source_dir=$1
    local backup_dir=$2

    echo "Starting backup from $source_dir"

    if [ ! -d "$source_dir" ]; then
        echo "Error: Source directory not found"
        return 1
    fi

    mkdir -p "$backup_dir"
    cp -r "$source_dir"/* "$backup_dir/"

    echo "Backup completed successfully"
}

# 主逻辑
main() {
    backup_files "/app/data" "/backup/data_$TIMESTAMP"
}

main
```

### Git

```git
# 常用 Git 命令

# 初始化仓库
git init
git clone <url>

# 基本操作
git add .
git commit -m "feat: add new feature"
git push origin main

# 分支操作
git checkout -b feature/new-feature
git merge main
git rebase main

# 查看状态
git status
git log --oneline --graph
git diff HEAD~1

# 储藏操作
git stash
git stash pop
git stash list

# 撤销操作
git reset --soft HEAD~1
git revert <commit>
git checkout -- <file>
```

### YAML

```yaml
# Docker Compose 配置示例
version: '3.8'

services:
  web:
    build: .
    ports:
      - "3000:3000"
    environment:
      - NODE_ENV=production
      - DATABASE_URL=postgres://user:pass@db:5432/myapp
    depends_on:
      - db
      - redis
    volumes:
      - ./data:/app/data
    restart: unless-stopped

  db:
    image: postgres:14-alpine
    environment:
      POSTGRES_USER: user
      POSTGRES_PASSWORD: pass
      POSTGRES_DB: myapp
    volumes:
      - pgdata:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    command: redis-server --appendonly yes
    volumes:
      - redisdata:/data

volumes:
  pgdata:
  redisdata:
```

### SQL

```sql
-- 创建用户表
CREATE TABLE users (
    id BIGINT PRIMARY KEY AUTO_INCREMENT,
    username VARCHAR(50) NOT NULL UNIQUE,
    email VARCHAR(100) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    status ENUM('active', 'inactive', 'banned') DEFAULT 'active',
    INDEX idx_username (username),
    INDEX idx_email (email)
);

-- 查询用户及其订单
SELECT
    u.id,
    u.username,
    u.email,
    COUNT(o.id) AS order_count,
    SUM(o.total_amount) AS total_spent
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = 'active'
  AND u.created_at >= DATE_SUB(NOW(), INTERVAL 1 YEAR)
GROUP BY u.id, u.username, u.email
HAVING COUNT(o.id) > 0
ORDER BY total_spent DESC
LIMIT 10;
```

---

## 4. 数学公式

### 行内公式

这是行内公式：$E = mc^2$，这是勾股定理 $a^2 + b^2 = c^2$，这是欧拉公式 $e^{i\pi} + 1 = 0$。

求根公式：$x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$

### 块级公式

**麦克斯韦方程组：**

$$
\nabla \times \mathbf{E} = -\frac{\partial \mathbf{B}}{\partial t}
$$

$$
\nabla \times \mathbf{B} = \mu_0 \mathbf{J} + \mu_0 \varepsilon_0 \frac{\partial \mathbf{E}}{\partial t}
$$

$$
\nabla \cdot \mathbf{E} = \frac{\rho}{\varepsilon_0}
$$

$$
\nabla \cdot \mathbf{B} = 0
$$

**矩阵：**

$$
\mathbf{A} = \begin{pmatrix}
a_{11} & a_{12} & a_{13} \\
a_{21} & a_{22} & a_{23} \\
a_{31} & a_{32} & a_{33}
\end{pmatrix}
$$

**分段函数：**

$$
f(x) = \begin{cases}
x & \text{if } x > 0 \\
0 & \text{if } x = 0 \\
-x & \text{if } x < 0
\end{cases}
$$

**求和与积分：**

$$
\sum_{i=1}^{n} i^2 = \frac{n(n+1)(2n+1)}{6}
$$

$$
\int_{0}^{\infty} e^{-x^2} dx = \frac{\sqrt{\pi}}{2}
$$

---

## 5. 列表

### 无序列表

- 苹果
- 香蕉
- 橙子
  - 赣南脐橙
  - 血橙
- 葡萄

### 有序列表

1. 第一步
2. 第二步
3. 第三步
   - 子步骤 A
   - 子步骤 B

### 任务列表

- [x] 完成项目初始化
- [x] 实现用户登录
- [ ] 实现文章发布
- [ ] 实现评论功能
- [ ] 添加数据统计

---

## 6. 表格

| 语言 | 创始人 | 年份 | 特点 |
|:-----|:-------|:-----|:-----|
| Python | Guido van Rossum | 1991 | 简洁易读 |
| Go | Robert Griesemer, Rob Pike, Ken Thompson | 2009 | 高并发 |
| Java | James Gosling | 1995 | 跨平台 |
| C | Dennis Ritchie | 1972 | 系统编程 |
| C++ | Bjarne Stroustrup | 1985 | 面向对象 |

---

## 7. 链接和图片

[百度](https://www.baidu.com)

![示例图片](https://via.placeholder.com/800x400)

---

## 8. 特殊字符

- 箭头：→ ← ↑ ↓ ⇒ ⇐ ⇑ ⇓
- 数学符号：∞ ∑ ∏ ∫ ∂ ∆ ∇ ± × ÷ ≠ ≤ ≥ ≈ ≡
- 希腊字母：α β γ δ ε ζ η θ ι κ λ μ ν ξ ο π ρ σ τ υ φ χ ψ ω
- 其他：★ ☆ ♠ ♣ ♥ ♦ ✓ ✗ © ® ™ § ¶ † ‡

---

## 9. 脚注

这里有一个脚注[^1]，还有另一个脚注[^2]。

[^1]: 这是第一个脚注的内容。
[^2]: 这是第二个脚注的内容，可以包含多行文字。

---

*文档结束*
