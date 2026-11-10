
-- the lines of a frame and the words of a line, without string patterns (the sandbox has none)
local function split(s, sep)
  local out, from = {}, 1
  for i = 1, #s do
    if s:sub(i, i) == sep then
      if i > from then out[#out + 1] = s:sub(from, i - 1) end
      from = i + 1
    end
  end
  if from <= #s then out[#out + 1] = s:sub(from) end
  return out
end
local function circles(frame)
  local out = {}
  for _, line in ipairs(split(frame, '\n')) do
    local w = split(line, ' ')
    if w[1] == 'circle' then out[#out + 1] = { x = tonumber(w[2]), y = tonumber(w[3]) } end
  end
  return out
end
local function cell(r, c) return 'c_' .. r .. '_' .. c end
local function ball(r, c) return get(cell(r, c), 'ball') end
local function count()
  local n = 0
  for r = 1, 9 do for c = 1, 9 do if ball(r, c) > 0 then n = n + 1 end end end
  return n
end
local function clear()
  for r = 1, 9 do for c = 1, 9 do set(cell(r, c), 'ball', 0) end end
end
local function click(r, c)
  local x, y = 184 + (c - 1) * 48 + 24, 120 + (r - 1) * 48 + 24
  mouse(x, y, false) step(2)
  mouse(x, y, true) press('mouse') step(2)
  mouse(x, y, false) step(3)
end
local function start()
  step(2)
  assert(get('app', 'screen') == 0, 'the game starts at the menu, screen is ' .. tostring(get('app', 'screen')))
  press('enter') step(3)
  assert(get('app', 'screen') == 1, 'enter on the menu starts the game, screen is ' .. tostring(get('app', 'screen')))
end
start()
clear() set(cell(1, 1), 'ball', 1) step()
click(1, 1)
assert(get('app', 'sel_row') == 1 and get('app', 'sel_col') == 1, 'a click on a ball selects it')
click(1, 5)
assert(ball(1, 5) == 1, 'the ball moved to the empty cell')
assert(get('app', 'sel_row') == 0, 'the selection is gone after a move')
assert(count() == 4, 'a move brings 3 new balls: 4 on the board, found ' .. count())
-- no path: a ball closed in a corner stays
clear() set(cell(1, 1), 'ball', 1) set(cell(1, 2), 'ball', 2) set(cell(2, 1), 'ball', 3) step()
click(1, 1) click(5, 5)
assert(ball(1, 1) == 1 and ball(5, 5) == 0, 'no path, no move')
assert(count() == 3, 'no move, no new balls, found ' .. count())
assert(get('app', 'sel_row') == 1 and get('app', 'sel_col') == 1, 'the ball stays selected when there is no path')
-- a click on another ball selects that one
click(1, 2)
assert(get('app', 'sel_row') == 1 and get('app', 'sel_col') == 2, 'another ball is selected instead')
-- a path around a wall
clear() set(cell(5, 1), 'ball', 1)
for r = 1, 8 do set(cell(r, 2), 'ball', (r % 2) + 2) end
step() click(5, 1) click(5, 9)
assert(ball(5, 9) == 1 and count() == 12, 'the ball goes around the wall: 9 and 3 new, found ' .. count())
-- an almost full board takes what is left
clear()
for r = 1, 9 do for c = 1, 9 do set(cell(r, c), 'ball', ((c + 2 * r) % 7) + 1) end end
set(cell(9, 9), 'ball', 0) set(cell(9, 8), 'ball', 0)
step() click(9, 7) click(9, 9)
assert(count() == 81, 'two free cells are both filled, found ' .. count())
