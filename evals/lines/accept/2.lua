
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
assert(count() == 5, 'a new game has 5 balls, it has ' .. count())
for r = 1, 9 do for c = 1, 9 do local b = ball(r, c) assert(b >= 0 and b <= 7 and b == math.floor(b), 'ball of a cell is 0..7') end end
assert(#circles(draw()) >= 5, 'the balls are drawn as circles, found ' .. #circles(draw()))
-- a ball is drawn in the middle of its cell
clear() set(cell(2, 3), 'ball', 4) step()
local found = false
for _, c in ipairs(circles(draw())) do
  if math.abs(c.x - (184 + 2 * 48 + 24)) < 1.5 and math.abs(c.y - (120 + 1 * 48 + 24)) < 1.5 then found = true end
end
assert(found, 'the ball of c_2_3 is a circle at 304, 192')
press('escape') step(3)
assert(get('app', 'screen') == 0, 'escape goes back')
press('enter') step(3)
assert(count() == 5, 'Play again starts a new game with 5 balls, it has ' .. count())
