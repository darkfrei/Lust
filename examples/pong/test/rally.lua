-- an idle left player loses points, the ball stays on the field, the keys move the paddle
step(600)
assert(get('right', 'score') >= 1, 'no point in 10 s')
local x = get('ball', 'x')
assert(x >= 0 and x <= 800, 'ball out: ' .. x)
keys('up') step(120) keys()
assert(get('left', 'y') == 45, 'paddle not at the top: ' .. get('left', 'y'))
