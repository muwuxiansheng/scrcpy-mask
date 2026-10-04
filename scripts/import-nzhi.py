"""Generate editable baseline profiles; never modifies the original N-zhi files."""
import argparse
import json
import re
from pathlib import Path

ALIASES = {'空格键': 'Space', 'Tab键': 'Tab', '左Shift键': 'ShiftLeft',
           '左Ctrl键': 'ControlLeft', '左Alt键': 'AltLeft', '大写键': 'CapsLock',
           'ESC键': 'Escape', 'Esc键': 'Escape', '回车键': 'Enter', '~键': 'Backquote',
           '鼠标左键': 'M-Left', '鼠标右键': 'M-Right', '鼠标中键': 'M-Middle',
           '中键上滚': 'ScrollUp', '中键下滚': 'ScrollDown',
           '鼠标前进键': 'M-Forward', '鼠标后退键': 'M-Back'}


def binding(key):
    if key in ALIASES:
        return ALIASES[key]
    if re.fullmatch(r'[A-Za-z]键', key):
        return 'Key' + key[0].upper()
    if re.fullmatch(r'[0-9]键', key):
        return 'Digit' + key[0]
    return None


def convert(source, width, height):
    raw = source.read_bytes()
    try:
        text = raw.decode('utf-8-sig')
    except UnicodeDecodeError:
        text = raw.decode('gbk')
    entries = []
    section = False
    for line in text.splitlines():
        line = line.strip()
        if line.startswith('['):
            section = line == '[键位]'
        elif section and '=' in line and '|' in line:
            label, code = line.split('=', 1)
            if code.isdigit() and len(code) >= 14:
                entries.append((label, label.split('|', 1)[1], code))
    mappings, skipped, seen = [], [], set()

    def pos(code):
        return {'x': round(int(code[6:10]) * width / 1000),
                'y': round(int(code[10:14]) * height / 1000)}

    def add(kind, note, point, key, pointer, **fields):
        mappings.append(dict(type=kind, id=f'nzhi-{len(mappings)+1}', note=note,
                             position=point, bind=key, pointer_id=pointer, **fields))

    center = next((e for e in entries if e[2].startswith('311')), None)
    if center:
        cpos = pos(center[2])
        directions = [pos(e[2]) for e in entries if e[2][:3] in ('314', '315', '316', '317')]
        dx = max((abs(p['x']-cpos['x']) for p in directions), default=round(width*.05))
        dy = max((abs(p['y']-cpos['y']) for p in directions), default=round(height*.05))
        add('DirectionPad', 'WASD 移动（请在训练场校准）', cpos,
            dict(type='Button', up=['KeyW'], down=['KeyS'], left=['KeyA'], right=['KeyD']),
            1, initial_duration=0, max_offset_x=dx, max_offset_y=dy,
            enable_randomization=False, random_offset_x=0, random_offset_y=0,
            jitter_offset_x=0, jitter_offset_y=0, up_boost_key=None, up_boost_scale=1.4)
    view = next((e for e in entries if e[2].startswith('40')), None)
    add('Fps', 'F8 切换鼠标视角／释放鼠标；灵敏度初值 0.8',
        pos(view[2]) if view else {'x': round(width*.65), 'y': round(height*.45)},
        ['F8'], 0, sensitivity_x=.8, sensitivity_y=.8,
        max_offset_x=0, max_offset_y=0, touch_mode={'type': 'single', 'interval': 0})
    for label, key, code in entries:
        if code.startswith('31') or code.startswith('40'):
            if not code.startswith(('311','314','315','316','317','40')):
                skipped.append({'label': label, 'code': code, 'reason': '额外轮盘行为未推断'})
            continue
        b = binding(key)
        if not b or b in seen:
            skipped.append({'label': label, 'code': code, 'reason': '未知按键或同键额外映射，需手工配置'})
            continue
        point = pos(code)
        if not (0 <= point['x'] < width and 0 <= point['y'] < height):
            skipped.append({'label': label, 'code': code, 'reason': '坐标超出屏幕，需校准'})
            continue
        seen.add(b)
        if b == 'M-Left':
            add('Fire', label, point, [b], 2, preserve_fps_control=True,
                sensitivity_x=.8, sensitivity_y=.8, random_offset_x=0, random_offset_y=0)
        else:
            add('SingleTap', label, point, [b], len(mappings)+3,
                duration=50, sync=b=='M-Right', random_offset_x=0, random_offset_y=0)
    return dict(version='0.0.1', original_size=dict(width=width, height=height), mappings=mappings), skipped


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, default=Path('D:/Nzhi'))
    parser.add_argument('--output', type=Path, default=Path('D:/AndroidTools/profiles'))
    parser.add_argument('--width', type=int, default=3392)
    parser.add_argument('--height', type=int, default=2400)
    args = parser.parse_args()
    if args.width <= 0 or args.height <= 0:
        parser.error('Screen dimensions must be positive')
    args.output.mkdir(parents=True, exist_ok=True)
    report = {}
    for source in sorted(args.source.glob('键位-*-官方初始键位.*')):
        profile, skipped = convert(source, args.width, args.height)
        target = args.output / (source.stem + '-初始适配.json')
        target.write_text(json.dumps(profile, ensure_ascii=False, indent=2), encoding='utf-8')
        report[target.name] = dict(source=str(source), mappings=len(profile['mappings']), skipped=skipped,
                                 note='仅转换坐标和基础输入。原格式后缀行为未解析；实际游戏 HUD 需校准。')
        print(target.name, len(profile['mappings']))
    (args.output/'转换说明.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
