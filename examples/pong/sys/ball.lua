-- the ball: walls, paddles, points
local W, H, R, MAX = 800, 600, 8, 900

-- from the middle, to the left and to the right in turn
local function serve()
  local n = ecs.get('left', 'score') + ecs.get('right', 'score')
  ecs.set('ball', 'x', W / 2)
  ecs.set('ball', 'y', H / 2)
  ecs.set('ball', 'vx', n % 2 == 0 and 300 or -300)
  ecs.set('ball', 'vy', 180)
end

-- paddle p sends the ball back, faster, and steeper the farther from its middle
local function bounce(p, x, y, vx, vy)
  local py, h = ecs.get(p, 'y'), ecs.get(p, 'h') / 2
  if math.abs(y - py) > h + R then return x, vx, vy end
  vx = math.max(-MAX, math.min(MAX, -vx * 1.05))
  vy = math.max(-MAX, math.min(MAX, vy + (y - py) * 4))
  return ecs.get(p, 'x') + (vx > 0 and 5 + R or -5 - R), vx, vy
end

return function(dt)
  local vx, vy = ecs.get('ball', 'vx'), ecs.get('ball', 'vy')
  local x = ecs.get('ball', 'x') + vx * dt
  local y = ecs.get('ball', 'y') + vy * dt
  if y < R then y, vy = 2 * R - y, -vy end
  if y > H - R then y, vy = 2 * (H - R) - y, -vy end
  local lx, rx = ecs.get('left', 'x'), ecs.get('right', 'x')
  if vx < 0 and x - R <= lx + 5 and x + R >= lx - 5 then
    x, vx, vy = bounce('left', x, y, vx, vy)
  elseif vx > 0 and x + R >= rx - 5 and x - R <= rx + 5 then
    x, vx, vy = bounce('right', x, y, vx, vy)
  end
  if x < 0 then
    ecs.set('right', 'score', ecs.get('right', 'score') + 1)
    serve()
  elseif x > W then
    ecs.set('left', 'score', ecs.get('left', 'score') + 1)
    serve()
  else
    ecs.set('ball', 'x', x)
    ecs.set('ball', 'y', y)
    ecs.set('ball', 'vx', vx)
    ecs.set('ball', 'vy', vy)
  end
end
