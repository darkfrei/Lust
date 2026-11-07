return {
  types = { x = 'num', y = 'num', vx = 'num', vy = 'num', h = 'num', speed = 'num', score = 'num' },
  entities = {
    ball = { x = 400, y = 300, vx = 300, vy = 180 },
    left = { x = 30, y = 300, h = 90, speed = 420, score = 0 },
    right = { x = 770, y = 300, h = 90, speed = 280, score = 0 },
    ['sys.paddles'] = { ord = 1 },
    ['sys.ball'] = { ord = 2 },
  },
}
