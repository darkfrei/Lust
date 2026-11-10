-- ui: buttons, labels, panels and menus for the screen of lust. it is part of every project.
-- use it from any code file:   local ui = ecs.call('lib.ui')
-- every size comes from gfx.measure, so a widget knows its width and height: it can be
-- centered, stacked and tested for a click. a widget is a plain table; the same table is
-- drawn (in a draw/ file) and tested (in a sys/ file), so build it the same way in both.
-- the mouse is input.mouse() -> x, y, down; a click is input.pressed('mouse').
-- the memory of the hovering mouse (update_menu) needs a game world (keep_lua).

local M = {}

M.theme = {
  text = '#ffffff', dim = '#8888aa', back = '#14141f',
  button = '#2b2b44', hover = '#3f3f66', down = '#5a5a8c', off = '#1c1c2a',
  border = '#7777aa', focus = '#ffd84a',
  size = 24, pad = 14,
}

-- the width and height of a text
function M.measure(text, size)
  return gfx.measure(text, size or M.theme.size)
end

-- a button: its text, where it is, and options: w, h (else as big as the text needs), size,
-- pad, enabled (false greys it out), id
function M.button(text, x, y, opts)
  opts = opts or {}
  local size = opts.size or M.theme.size
  local pad = opts.pad or M.theme.pad
  local tw, th = gfx.measure(text, size)
  return {
    text = text, x = x, y = y, size = size, pad = pad, id = opts.id,
    w = opts.w or (tw + 2 * pad), h = opts.h or (th + pad),
    tw = tw, th = th, enabled = opts.enabled ~= false,
  }
end

-- is the point inside the widget
function M.inside(b, px, py)
  return px >= b.x and px < b.x + b.w and py >= b.y and py < b.y + b.h
end

-- the mouse is over the button
function M.hover(b)
  local mx, my = input.mouse()
  return b.enabled and M.inside(b, mx, my)
end

-- the mouse is over the button and its key is down
function M.down(b)
  local mx, my, held = input.mouse()
  return b.enabled and held and M.inside(b, mx, my)
end

-- the button was clicked in this tick (call it in a sys/ file)
function M.clicked(b)
  if not b.enabled or not input.pressed('mouse') then
    return false
  end
  local mx, my = input.mouse()
  return M.inside(b, mx, my)
end

-- draws a button; focus = true marks the one the keys are on
function M.draw(b, focus)
  local t = M.theme
  local fill = t.button
  if not b.enabled then
    fill = t.off
  elseif M.down(b) then
    fill = t.down
  elseif M.hover(b) or focus then
    fill = t.hover
  end
  gfx.rect(b.x - 2, b.y - 2, b.w + 4, b.h + 4, focus and t.focus or t.border)
  gfx.rect(b.x, b.y, b.w, b.h, fill)
  gfx.text(b.x + (b.w - b.tw) / 2, b.y + (b.h - b.th) / 2, b.size, b.enabled and t.text or t.dim, b.text)
end

-- draws a text anchored at x: opts.align is 'left' (default), 'center' or 'right'; also
-- opts.size and opts.color. gives the width and the height of the text
function M.label(text, x, y, opts)
  opts = opts or {}
  local size = opts.size or M.theme.size
  local w, h = gfx.measure(text, size)
  local left = x
  if opts.align == 'center' then
    left = x - w / 2
  elseif opts.align == 'right' then
    left = x - w
  end
  gfx.text(left, y, size, opts.color or M.theme.text, text)
  return w, h
end

-- a filled rectangle with a border
function M.panel(x, y, w, h, color, border)
  gfx.rect(x - 2, y - 2, w + 4, h + 4, border or M.theme.border)
  gfx.rect(x, y, w, h, color or M.theme.back)
end

-- puts the buttons one under another, centered at cx, from top, all as wide as the widest.
-- gives the bottom edge
function M.stack(list, cx, top, gap)
  gap = gap or M.theme.pad
  local wide = 0
  for _, b in ipairs(list) do
    if b.w > wide then wide = b.w end
  end
  local y = top
  for _, b in ipairs(list) do
    b.w = wide
    b.x = cx - wide / 2
    b.y = y
    y = y + b.h + gap
  end
  return y - gap
end

-- puts the buttons side by side, centered at cx, from top. gives the right edge
function M.row(list, cx, top, gap)
  gap = gap or M.theme.pad
  local total = -gap
  for _, b in ipairs(list) do
    total = total + b.w + gap
  end
  local x = cx - total / 2
  for _, b in ipairs(list) do
    b.x = x
    b.y = top
    x = x + b.w + gap
  end
  return x - gap
end

-- a menu: a stack of buttons made from the labels. an item is a text, or a table
-- {text = '...', enabled = false}. options: size, pad, w (the least width), gap
function M.menu(items, cx, top, opts)
  opts = opts or {}
  local list = {}
  for i, item in ipairs(items) do
    local o = { size = opts.size, pad = opts.pad, w = opts.w, id = i }
    local text = item
    if type(item) == 'table' then
      text = item.text
      o.enabled = item.enabled
    end
    list[i] = M.button(text, 0, 0, o)
  end
  M.stack(list, cx, top, opts.gap)
  return list
end

-- the next enabled button from sel in a direction (1 or -1), around the ends
local function next_enabled(list, sel, dir)
  local n = #list
  for _ = 1, n do
    sel = (sel - 1 + dir) % n + 1
    if list[sel].enabled then
      return sel
    end
  end
  return sel
end

-- the tick side of a menu: up and down move the choice, a moving mouse too, enter or space
-- or a click chooses. gives the new selection, the index chosen (or nil) and the edge
-- (-1 or 1 when an arrow was pressed at an end and opts.wrap is false, else 0)
function M.update_menu(list, sel, opts)
  opts = opts or {}
  local n = #list
  if n == 0 then
    return 1, nil, 0
  end
  sel = math.max(1, math.min(n, sel or 1))
  local edge = 0
  if input.pressed('down') then
    if opts.wrap == false and sel == n then edge = 1 else sel = next_enabled(list, sel, 1) end
  end
  if input.pressed('up') then
    if opts.wrap == false and sel == 1 then edge = -1 else sel = next_enabled(list, sel, -1) end
  end
  local mx, my = input.mouse()
  if mx ~= M.last_x or my ~= M.last_y then
    M.last_x, M.last_y = mx, my
    for i, b in ipairs(list) do
      if b.enabled and M.inside(b, mx, my) then sel = i end
    end
  end
  local chosen = nil
  if (input.pressed('enter') or input.pressed('space')) and list[sel].enabled then
    chosen = sel
  end
  for i, b in ipairs(list) do
    if M.clicked(b) then
      sel = i
      chosen = i
    end
  end
  return sel, chosen, edge
end

-- the draw side of a menu
function M.draw_menu(list, sel)
  for i, b in ipairs(list) do
    M.draw(b, i == sel)
  end
end

-- the words of a text; a new line is a word of its own
local function words(text)
  local list, word = {}, ''
  for i = 1, #text do
    local c = string.sub(text, i, i)
    if c == ' ' or c == '\n' then
      if word ~= '' then
        list[#list + 1] = word
        word = ''
      end
      if c == '\n' then list[#list + 1] = '\n' end
    else
      word = word .. c
    end
  end
  if word ~= '' then list[#list + 1] = word end
  return list
end

-- breaks a text into lines no wider than w
function M.wrap(text, w, size)
  size = size or M.theme.size
  local lines, line = {}, ''
  for _, word in ipairs(words(text)) do
    if word == '\n' then
      lines[#lines + 1] = line
      line = ''
    else
      local joined = line == '' and word or (line .. ' ' .. word)
      if line ~= '' and gfx.measure(joined, size) > w then
        lines[#lines + 1] = line
        line = word
      else
        line = joined
      end
    end
  end
  if line ~= '' then lines[#lines + 1] = line end
  return lines
end

-- draws a text in lines no wider than w, from x, y. options: size, color. gives its height
function M.paragraph(text, x, y, w, opts)
  opts = opts or {}
  local size = opts.size or M.theme.size
  local lines = M.wrap(text, w, size)
  local _, h = gfx.measure('M', size)
  for i, line in ipairs(lines) do
    gfx.text(x, y + (i - 1) * h, size, opts.color or M.theme.text, line)
  end
  return #lines * h
end

return function()
  return M
end
