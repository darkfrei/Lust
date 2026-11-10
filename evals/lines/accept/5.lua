
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
clear()
for r = 1, 9 do for c = 1, 9 do set(cell(r, c), 'ball', ((c + 2 * r) % 7) + 1) end end
set(cell(9, 9), 'ball', 0) set(cell(9, 8), 'ball', 0)
set('app', 'score', 7) set('app', 'best', 3) step()
click(9, 7) click(9, 9)
step(5)
assert(get('app', 'screen') == 2, 'a full board ends the game, screen is ' .. get('app', 'screen'))
assert(get('app', 'best') == 7, 'the best score is 7 now, is ' .. get('app', 'best'))
local f = draw()
assert(f:find_plain('Игра окончена'), 'the words of the end are drawn')
assert(f:find_plain('7') and not f:find_plain('7.0'), 'the score of the game is drawn as a whole number')
press('enter') step(3)
assert(get('app', 'screen') == 0, 'enter goes to the menu')
local m = draw()
assert(m:find_plain('Лучший счёт') and m:find_plain('7'), 'the menu shows the best score')
press('enter') step(3)
assert(get('app', 'screen') == 1 and get('app', 'score') == 0 and count() == 5, 'a new game: score 0, 5 balls')
assert(get('app', 'best') == 7, 'the best score stays')
-- a smaller score does not lower the best
clear()
for r = 1, 9 do for c = 1, 9 do set(cell(r, c), 'ball', ((c + 2 * r) % 7) + 1) end end
set(cell(9, 9), 'ball', 0) set(cell(9, 8), 'ball', 0) set('app', 'score', 2) step()
click(9, 7) click(9, 9) step(5)
assert(get('app', 'screen') == 2 and get('app', 'best') == 7, 'a smaller score keeps the best')
press('escape') step(3)
assert(get('app', 'screen') == 0, 'escape goes to the menu too')
