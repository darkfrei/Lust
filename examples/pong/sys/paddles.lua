-- the left paddle follows the keys (w s or up down), the right one follows the ball
local H = 600

local function move(p, dir, dt)
  local h = ecs.get(p, 'h') / 2
  local y = ecs.get(p, 'y') + dir * ecs.get(p, 'speed') * dt
  ecs.set(p, 'y', math.max(h, math.min(H - h, y)))
end

return function(dt)
  local dir = 0
  if input.down('up') or input.down('w') then dir = dir - 1 end
  if input.down('down') or input.down('s') then dir = dir + 1 end
  move('left', dir, dt)
  local d = ecs.get('ball', 'y') - ecs.get('right', 'y')
  local ai = 0
  if d > 10 then ai = 1 elseif d < -10 then ai = -1 end
  move('right', ai, dt)
end
