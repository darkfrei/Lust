
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
assert(get('app', 'score') == 0, 'a new game has score 0')
-- a row of 5
clear() for c = 1, 4 do set(cell(3, c), 'ball', 2) end set(cell(5, 5), 'ball', 2) step()
click(5, 5) click(3, 5)
assert(count() == 0, 'a row of 5 is removed and no new balls come, found ' .. count())
assert(get('app', 'score') == 5, 'five balls give 5 points, score ' .. get('app', 'score'))
-- a diagonal
clear() for i = 1, 4 do set(cell(i, i), 'ball', 3) end set(cell(9, 5), 'ball', 3) step()
click(9, 5) click(5, 5)
assert(count() == 0, 'a diagonal of 5 is removed, found ' .. count())
assert(get('app', 'score') == 10, 'score 10, is ' .. get('app', 'score'))
-- the other diagonal, 6 in a line
clear() for i = 1, 5 do set(cell(i, 10 - i), 'ball', 4) end set(cell(6, 4), 'ball', 0) set(cell(9, 9), 'ball', 4)
set(cell(3, 7), 'ball', 0) set(cell(7, 3), 'ball', 4) step()
-- now (1,9) (2,8) _ (4,6) (5,5) _ (7,3): put the ball of (9,9) into (3,7): 1,2,3,4,5 in a line
click(9, 9) click(3, 7)
assert(ball(1, 9) == 0 and ball(5, 5) == 0 and ball(3, 7) == 0, 'the other diagonal is removed')
assert(get('app', 'score') == 15, 'score 15, is ' .. get('app', 'score'))
-- a column
clear() for r = 1, 4 do set(cell(r, 8), 'ball', 5) end set(cell(9, 1), 'ball', 5) step()
click(9, 1) click(5, 8)
assert(count() == 0, 'a column of 5 is removed, found ' .. count())
-- four in a row stay, and new balls come
clear() set('app', 'score', 0) for c = 1, 3 do set(cell(7, c), 'ball', 6) end set(cell(1, 9), 'ball', 6) step()
click(1, 9) click(7, 4)
assert(ball(7, 1) == 6 and ball(7, 4) == 6, 'four in a row stay')
assert(count() == 7, '4 and 3 new balls, found ' .. count())
-- the score is drawn as a whole number
set('app', 'score', 12) step()
local f = draw()
assert(f:find_plain('12') and not f:find_plain('12.0'), 'the score 12 is drawn as a whole number')
