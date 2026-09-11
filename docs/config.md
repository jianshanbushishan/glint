# Glint JSON 配置

主配置为配置目录中的 `config.json`，使用标准 JSON，不支持注释和尾逗号。加载时同时校验 `gestures.json` 和 `overrides.json`；任一文件无效都会保留上一份有效配置。完整预置见 [config/config.json](../config/config.json)，包含 26 个全局绑定。

## 基础配置与动作

开机自动启动在“常规 → 启动”中设置，默认关闭，切换立即保存，无需点击“应用”。此选项保存在当前用户的 Windows 启动项（`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名 `Glint`），不写入 JSON，也不需要管理员权限。启动项指向同目录的 `glint.exe` 并携带当前配置目录；每个 Windows 用户只有一个 Glint 启动项。移动程序或更换配置目录后，请重新开启开关以更新路径；删除程序前可先关闭开关。

```json
{
  "version": 1,
  "stroke_button": "right",
  "pen": {"color":"#69E0C3","invalid_color":"#9CA3AF","width":4,"opacity":0.85},
  "logging": {"level":"info"},
  "threshold": 0.82,
  "min_distance": 24,
  "excluded": [],
  "packages": [{
    "id":"global", "name":"通用", "match":[".*"],
    "actions":[
      {"id":"copy","name":"复制","gesture":"up_right","action":{"type":"keys","keys":"CTRL+C"}},
      {"id":"minimize","name":"最小化","gesture":"up_down","action":{"type":"window","operation":"minimize"}},
      {"id":"files","name":"打开文件夹","gesture":"clockwise","action":{"type":"launch","program":"explorer.exe","args":["C:\\Users"]}}
    ]
  }]
}
```

顶层字段可省略，使用示例中的数值默认值；`excluded`、`packages` 和 `applications` 默认为空，`gestures` 默认为内置 28 个模板。动作须包含 `id`、`name`、`gesture`、`action`，动作包须包含 `id`、`name`、`match`、`actions`。动作仅支持 `keys`、`window`、`launch` 三种类型，启动参数 `args` 可省略。

`stroke_button` 接受 `left`、`right`、`middle`、`x1`、`x2`。笔宽度为 1–32，透明度和阈值为 0–1，最小移动距离为 1–10000 像素。未知字段、重复 ID、重复包内手势绑定、不存在的手势、无效正则和不支持的动作都会使加载失败。

`pen.color` 设置正常轨迹颜色，`pen.invalid_color` 设置无效轨迹颜色，省略时默认为灰色 `#9CA3AF`，旧配置无需修改。两种颜色都使用 `#RRGGBB` 格式，共用线宽和不透明度；设置界面提供并排的颜色选择器。

绘制过程中会按当前应用的有效绑定检查手势前缀，包括仍可追加按钮或滚轮的组合手势。一旦所有候选都被排除，整条轨迹变为无效颜色，本次手势永久失效：继续移动不能恢复，松开触发键、追加按钮或滚轮均不执行动作。松开后重新开始手势会重置判定。短按点击与新手势录制不受此前缀判定限制。前缀检查使用带容差的形状相似度，因此这是在原有完整轨迹评分之外增加的接受条件；不再允许通过后续很长的轨迹抵消已经判废的开头。

## 日志

日志通过 Rust 标准 `log` crate 写入配置目录中的 `glint.log`，超过 1 MiB 后轮换为 `glint.previous.log`。设置窗口单独记录到 `glint-settings.log`，按同样规则轮换为 `glint-settings.previous.log`。`logging.level` 接受 `off`、`error`、`warn`、`info`、`debug`、`trace`，默认 `info`。常规设置中应用日志级别后立即生效并持久化；直接修改基础配置也支持热重载，但界面保存的覆盖值优先。动作执行记录为 `debug`，需要查看执行了哪些动作时选择 `debug` 或 `trace`。启动失败仍可从 `startup-error.log` 排查。

## 匹配与应用

`match` 和 `excluded` 使用不区分大小写的 Rust regex 语法。匹配对象通常为进程完整路径，排除列表优先。未配置显式应用的进程从后向前查找匹配的动作包；当前包没有该手势时继续回退。

`applications` 每项包含 `id`、`name`、`process_path`，以及可省略的 `disabled`（默认 `false`）和 `inherit_global`（默认 `true`）。应用 ID 不能为 `global`，关联同 ID 动作包；缺失时创建空包，已有包的名称和匹配表达式由应用元数据决定，动作保留。

```json
{
  "applications":[{
    "id":"editor","name":"编辑器","process_path":"C:\\Apps\\Editor\\editor.exe",
    "disabled":false,"inherit_global":true
  }],
  "packages":[{
    "id":"editor","name":"编辑器","match":[],
    "actions":[{"id":"save","name":"保存","gesture":"down","action":{"type":"keys","keys":"CTRL+S"}}]
  }]
}
```

路径须为完整 Windows `.exe` 路径，支持盘符和 UNC，不要求文件当前存在。加载和保存时统一分隔符、折叠点段，拒绝越过根目录或含歧义的路径。规范化后精确匹配且忽略大小写；等价路径不能重复配置。应用专属手势优先，未命中时依据 `inherit_global` 决定是否继承全局；`disabled` 会在捕获前放行该应用的鼠标输入。

## 界面覆盖与录制

`gestures.json` 是模板数组，以 ID 覆盖基础模板，新 ID 追加。界面录制始终使用新 ID，应用对应绑定后生效，不改写其他绑定共享的模板。

```json
[{"id":"my_left","name":"我的左划","points":[{"x":100,"y":0},{"x":0,"y":0}]}]
```

`overrides.json` 保存界面增量修改。可选 `pen`、`stroke_button`、`logging` 覆盖基础字段；`application_overrides` 按 ID 新增或更新应用，`removed_applications` 删除应用和对应包；`action_overrides` 按包 ID 和动作 ID 新增或替换动作，`removed_actions` 删除指定动作。未覆盖的动作继续跟随基础配置修改。首次保存全局动作时，若基础配置没有 `global` 包，会创建最低优先级的全局包。

```json
{
  "logging":{"level":"debug"},
  "action_overrides":[{
    "package_id":"global",
    "action":{"id":"copy","name":"复制","gesture":"up","action":{"type":"keys","keys":"CTRL+C"}}
  }],
  "removed_actions":[{"package_id":"global","action_id":"back"}]
}
```

可选 `packages` 字段整体替换基础包列表；界面使用增量记录。合并顺序为包列表替换、应用删除和元数据合并、生成应用包、动作删除、动作覆盖。界面一次提交当前作用域的元数据和动作增删，完整校验和写入成功才生效；失败保留当前配置。移除应用时清理对应的动作覆盖。`settings.json` 单独保存设置窗口外观。

加载顺序为解析基础 JSON、合并录制模板、规范化应用和生成包、校验基础配置、应用覆盖、校验最终配置。禁用应用的精确规则仅加入内存中的排除列表。基础配置的无效动作不能被覆盖隐藏。

## 内置手势与识别

| ID | 形状 | ID | 形状 |
| --- | --- | --- | --- |
| `left` | 向左 | `right` | 向右 |
| `up` | 向上 | `down` | 向下 |
| `up_left` | 左上斜线 | `up_right` | 右上斜线 |
| `down_left` | 左下斜线 | `down_right` | 右下斜线 |
| `left_up` | 左再上 | `left_down` | 左再下 |
| `right_up` | 右再上 | `right_down` | 右再下 |
| `up_down` | 上再下 | `down_up` | 下再上 |
| `left_right` | 左再右 | `right_left` | 右再左 |
| `up_then_left` | 上再左 | `up_then_right` | 上再右 |
| `down_then_left` | 下再左 | `down_then_right` | 下再右 |
| `down_then_right_then_down` | 下右下 | `right_then_up_then_left` | 右上左 |
| `up_then_right_then_up` | 上右上 | `up_then_right_then_down_then_left` | 上右下左 |
| `v` | V 字 | `inverted_v` | 倒 V 字 |
| `clockwise` | 从顶部起笔的顺时针圆 | `counterclockwise` | 从顶部起笔的逆时针圆 |

引擎只使用当前进程有效绑定引用的轨迹作为识别候选。普通手势只比较普通轨迹绑定，组合手势只比较同一附加输入后缀的绑定；未绑定、取消录制留下或仅供其他应用使用的模板不会参与本次识别。同分时按动作包优先级决定，应用专属先于继承的全局，同包内保持模板顺序；因此应用中新录制的重复轨迹可覆盖全局动作，又不改变其他应用的识别结果。

识别将轨迹按弧长等距重采样，然后比较局部单位方向向量，忽略平移和统一缩放，保留旋转、起笔位置和绘制顺序。小窗口降低鼠标抖动影响。得分为平均方向点积裁剪到 0–1；最高分低于 `threshold` 时没有命中。圆形需要按模板从顶部起笔；当前算法不做循环起点搜索。`recognize` 是纯函数；最小距离是捕获层的判定，识别函数自身不依赖屏幕像素大小。

## 特殊手势

`gesture` 除模板 ID 外，也支持 `wheel_up`、`wheel_down`、`left_click`、`right_click`、`middle_click`、`x1_click`、`x2_click`。这些动作在按住触发键时滚动滚轮或点击另一按钮触发，不需要轨迹模板。触发按钮本身不能同时作为另一按钮点击，例如以右键触发时 `right_click` 不会发生。

也支持 `轨迹模板ID+附加输入ID`，例如 `right_down+left_click` 表示先右再下，然后按下左键。后缀可以是上述五种按钮点击 ID，或 `wheel_up`、`wheel_down`，不接受多重后缀。组合前缀必须引用已有轨迹模板。按钮点击在按下时分派，滚轮在滚动时分派；已有轨迹时仅匹配相同后缀的组合绑定，没有绑定时不回退到直接输入或普通轨迹动作。组合输入后松开触发键不再执行普通轨迹。按钮组合会消耗当前轨迹；滚轮组合保留轨迹，持续按住触发键可重复滚轮执行动作。录制模式下不会执行按钮或滚轮动作。

