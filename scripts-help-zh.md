# 📜 脚本语法规则简介

## 🧩 变量与类型

* 动态类型系统，支持：

  * `Int`（i64）
  * `Bool`（布尔值）
  * `Str`（字符串）
* 变量需通过 `let` 声明并赋值后使用

  ```js
  let x = 100
  ```
* 支持重新赋值（需先声明）

  ```js
  x = 200
  ```
* 分号 `;` 可像 JavaScript 一样在多数行尾省略。若同一行写多条普通语句，仍需用 `;` 分隔。

---

## ➕ 运算符

### 算术运算

* `+` 加法
* `-` 减法
* `*` 乘法
* `/` 除法
* `%` 取模

### 比较运算

* `<` 小于
* `<=` 小于等于
* `>` 大于
* `>=` 大于等于
* `==` 等于
* `!=` 不等于

### 逻辑运算

* `&&` 与（and）
* `||` 或（or）
* `!` 非（not）

### 字符串拼接

* `+` 可用于连接字符串

```js
"Hello" + "World" // => "HelloWorld"
```
---

## 🧠 控制结构

### 条件分支

```js
if x > 10 { ... } else { ... }
```

### 循环

```js
while x > 0 { x = x - 1 }
```

---

## 🔢 内置常量

| 常量名          | 说明             |
| ------------ | -------------- |
| `ORIGINAL_W` | 配置区域的原始宽度      |
| `ORIGINAL_H` | 配置区域的原始高度      |
| `CURSOR_X`   | 鼠标指针在蒙版内的 X 坐标 |
| `CURSOR_Y`   | 鼠标指针在蒙版内的 Y 坐标 |
| `RawInputFlag` | 当前是否处于直控模式（`Bool`） |
| `FpsModeFlag` | 当前是否处于 FPS 模式（`Bool`） |

> 每次脚本重新执行时常量更新，执行期间为固定值。

---

## ⚙️ 内置函数

### `print(...)`

输出日志（参数自动转换为字符串）

```js
print("Value:", x); // 输出 "Value: 100"
```

### `wait(ms)`

暂停执行指定毫秒数

```js
wait(1000); // 等待 1 秒
```

### `tap(pointer_id, x, y, action?)`

模拟触摸事件

* `pointer_id`: 触控点 ID（非负整数）
* `x, y`: 相对坐标（相对于 `ORIGINAL_W` / `ORIGINAL_H`）
* `action`: `"down"`, `"up"`, `"move"`, `"default"`（默认触发 down 后 30ms up）

### `tap_random(pointer_id, x, y, offset_x, offset_y)`

随机落点的短点击，例如 `tap_random(54, 1462, 2240, 10, 10)`。
X/Y 分别在中心坐标 ±offset 范围内均匀取样，单位为手机原始坐标像素；偏移须为非负整数，0 表示该轴不偏移。落点限制在屏幕内，同一次点击的按下和 30ms 后抬起使用相同坐标。

### `swipe(pointer_id, interval, x1, y1, x2, y2, ...)`

模拟滑动操作

* `interval`: 相邻坐标间滑动时间（毫秒）
* 至少两组坐标点 (`x1, y1, x2, y2`)

### `send_key(key_name, action?, metastate?)`

发送按键事件

* `key_name`: 按键名（如 `"KEYCODE_HOME"`）
* `action`: `"down"`, `"up"`, `"default"`（默认按下并释放）
* `metastate`: 修饰键（如 `"META_SHIFT_ON"`）

### `paste_text(text)`

粘贴指定文本到设备

```js
paste_text("Hello from script!");
```

### `state_set(name, value, scope?)`

为当前 Script 映射保存一个共享状态值。

* `name`: 状态名（非空字符串）
* `value`: `Int`、`Bool` 或 `Str`
* `scope`: 可选的映射 ID（非空字符串），省略时使用当前映射。用于跨映射同步，例如数字键钩子中 `state_set("next_item", 4, "fenghuo-wheel-cycle")`，让滚轮下次选择第4项。

### `state_get(name, default_value, scope?)`

读取当前 Script 映射的共享状态值。如果值不存在，返回 `default_value`。
可选 `scope` 可读取指定映射 ID 的状态；未指定时保持原有的独立状态行为。

### `state_has(name)`

返回共享状态值是否存在。

### `state_delete(name)`

删除一个共享状态值，并返回是否实际删除了值。

### `state_clear()`

清空当前 Script 映射的所有共享状态值。

> `state_*` 的值会在同一个 Script 映射的按下、按住、抬起脚本之间共享。其他 Script 映射使用独立状态。值会持续到被删除、清空，或脚本运行时状态被重新创建。

### `enter_fps(id)`

进入指定 FPS 映射的 FPS 模式。

* `id`: FPS 映射的 `id`

### `recover_fps(id)`

恢复控制状态，例如 `recover_fps("fenghuo-view")`。停止当前映射以释放移动、开火、自由视角、长按等触点，取消滚轮队列，关闭手机指针，释放脚本遗留触点，清空布局脚本状态，再进入指定FPS映射。旧脚本在下一次调用/触摸发送时取消。

不发送游戏返回键，不自动关闭地图/背包界面。建议先手动关闭游戏界面后按恢复键。清空脚本状态也会使滚轮循环回到第1项。键位可创建为“脚本”，只在按下脚本填入此函数。

### `exit_fps()`

退出 FPS 模式。

### `enter_phone_pointer()` / `exit_phone_pointer()`（本地修改版）

在 FPS 总开关开启时，明确进入手机指针模式或恢复 FPS 视角。
进入后鼠标悬浮移动，左键按下、拖动、松开对应手机触摸；指针从屏幕中心呼出。
重复进入或重复退出不会反向切换。退出时释放正在按住的指针触点。
如果 FPS 尚未开启，先用 `enter_fps(id)` 开启。常用于背包、地图的开关脚本。

### `enter_raw_input()`

进入原始输入模式。FPS 模式下会被忽略。

### `exit_raw_input()`

退出原始输入模式。

### `cancel_cast(id)`

使用指定取消技能映射取消当前技能。

* `id`: CancelCast 映射的 `id`

### `release_cast()`

直接释放当前技能，不经过取消技能位置。

---

## ⚠️ 错误处理

* 脚本会提示语法或运行时错误的具体位置和上下文：

  ```text
  error: Division by zero
   --> line 5, column 10 to line 5, column 15
    |
  5 |     10 / 0
    |      ^^^^^
  ```

---

## 🚫 限制与注意事项

* 不支持用户自定义函数
* 所有变量为 **全局作用域**（块内声明外部可访问）
* `send_key` 的 `key_name` 和 `metastate` 需符合
  [src/scrcpy/constant.rs](src/scrcpy/constant.rs) 中定义的枚举规范

---

## 💡 示例脚本

```rs
// 声明并初始化变量
let x = ORIGINAL_W / 2
let y = ORIGINAL_H / 2
let counter = 0

// 使用内置常量进行计算
print("Original size:", ORIGINAL_W, "x", ORIGINAL_H)
print("Cursor position:", CURSOR_X, CURSOR_Y)

// 条件语句示例
if CURSOR_X > ORIGINAL_W / 2 {
    print("Cursor is on the right side")
} else {
    print("Cursor is on the left side or middle")
}

// 循环示例
while counter < 3 {
    tap(counter, x, y)      // 在当前位置点击
    x = x + 100             // 更新变量
    counter = counter + 1
    wait(500)               // 等待一段时间
}

// 字符串操作示例
let message = "Hello" + " " + "World"
print(message)

// 滑动示例：从中心点向右上角滑动
swipe(0, 500, ORIGINAL_W/2, ORIGINAL_H/2, ORIGINAL_W/2 + 200, ORIGINAL_H/2 - 200)

// 粘贴文本示例
paste_text("Hello from script!")

// 按键示例
send_key("VolumeUp") // 按下并释放音量键

// 使用修饰键的示例
send_key("A", "default", "CTRL_ON")

// 控制按键时长的示例
send_key("Home", "down")
wait(100)
send_key("Home", "up")

// 使用逻辑运算符
let flag = true
if flag && counter > 0 {
    print("Flag is true and counter is positive")
}

if !flag || counter == 3 {
    print("Either flag is false or counter equals 3")
}

// 数值比较
if x > ORIGINAL_W / 2 && y < ORIGINAL_H / 2 {
    print("Position is in the upper right quadrant")
}
```
