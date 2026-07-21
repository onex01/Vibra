# Vibra Pong — Консольный Понг
# Два игрока, мяч, ракетки, очки

print "========================="
print "     VIBRA PONG GAME"
print "========================="
print ""
print "Automated demo match!"
print "Watch the AI play!"
print ""

var WIDTH = 40
var HEIGHT = 15
var PADDLE_SIZE = 3

# Позиции ракеток
var paddle1 = 6
var paddle2 = 6

# Мяч
var ball_x = 20
var ball_y = 7
var ball_dx = 1
var ball_dy = 1

# Очки
var score1 = 0
var score2 = 0
var rally = 0
var max_rallies = 20

var frame = 0
while frame < max_rallies * 10 {
    # Рендер
    var y = 0
    while y < HEIGHT {
        var line = ""
        var x = 0
        while x < WIDTH {
            if x == ball_x && y == ball_y {
                line = line + "O"
            } else if x == 1 && y >= paddle1 && y < paddle1 + PADDLE_SIZE {
                line = line + "["
            } else if x == WIDTH - 2 && y >= paddle2 && y < paddle2 + PADDLE_SIZE {
                line = line + "]"
            } else if x == WIDTH / 2 {
                if y % 2 == 0 {
                    line = line + ":"
                } else {
                    line = line + " "
                }
            } else if y == 0 || y == HEIGHT - 1 {
                line = line + "="
            } else {
                line = line + " "
            }
            x = x + 1
        }
        print line
        y = y + 1
    }

    print "P1: " + score1 + "  |  P2: " + score2 + "  |  Rally: " + rally

    # AI для ракетки 1 — следит за мячом
    if ball_y < paddle1 + 1 {
        paddle1 = paddle1 - 1
    }
    if ball_y > paddle1 + PADDLE_SIZE - 2 {
        paddle1 = paddle1 + 1
    }

    # AI для ракетки 2 — следит за мячом
    if ball_y < paddle2 + 1 {
        paddle2 = paddle2 - 1
    }
    if ball_y > paddle2 + PADDLE_SIZE - 2 {
        paddle2 = paddle2 + 1
    }

    # Ограничение ракеток
    if paddle1 < 1 { paddle1 = 1 }
    if paddle1 > HEIGHT - PADDLE_SIZE - 1 { paddle1 = HEIGHT - PADDLE_SIZE - 1 }
    if paddle2 < 1 { paddle2 = 1 }
    if paddle2 > HEIGHT - PADDLE_SIZE - 1 { paddle2 = HEIGHT - PADDLE_SIZE - 1 }

    # Движение мяча
    ball_x = ball_x + ball_dx
    ball_y = ball_y + ball_dy

    # Отскок от верхней/нижней стенки
    if ball_y <= 1 || ball_y >= HEIGHT - 2 {
        ball_dy = 0 - ball_dy
        beep 440 50
    }

    # Отскок от ракетки 1
    if ball_x == 2 && ball_y >= paddle1 && ball_y < paddle1 + PADDLE_SIZE {
        ball_dx = 1
        rally = rally + 1
        beep 660 50
    }

    # Отскок от ракетки 2
    if ball_x == WIDTH - 3 && ball_y >= paddle2 && ball_y < paddle2 + PADDLE_SIZE {
        ball_dx = 0 - 1
        rally = rally + 1
        beep 660 50
    }

    # Гол для P1 (мяч у правой стены)
    if ball_x >= WIDTH - 1 {
        score1 = score1 + 1
        beep 880 200
        ball_x = WIDTH / 2
        ball_y = HEIGHT / 2
        ball_dx = 0 - 1
        ball_dy = 1
        rally = 0
    }

    # Гол для P2 (мяч у левой стены)
    if ball_x <= 0 {
        score2 = score2 + 1
        beep 880 200
        ball_x = WIDTH / 2
        ball_y = HEIGHT / 2
        ball_dx = 1
        ball_dy = 1
        rally = 0
    }

    frame = frame + 1
}

print ""
print "========================="
print "    MATCH COMPLETE!"
print "  P1: " + score1 + "  P2: " + score2
if score1 > score2 {
    print "  Winner: Player 1!"
}
if score2 > score1 {
    print "  Winner: Player 2!"
}
if score1 == score2 {
    print "  Draw!"
}
print "========================="
print ""
beep 523 100
sleep 100
beep 659 100
sleep 100
beep 784 100
sleep 100
beep 0
