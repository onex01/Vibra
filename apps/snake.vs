# Vibra Snake — Консольная змейка
# Использует print для рендера网格, beep для звука

print "========================="
print "    VIBRA SNAKE GAME"
print "========================="
print ""
print "Controls: WASD to move"
print "Collect food (#) to grow!"
print ""

# Игровое поле: 20x40
var WIDTH = 40
var HEIGHT = 15

# Позиция змейки
var head_x = 20
var head_y = 7
var dir_x = 1
var dir_y = 0

# Еда
var food_x = 10
var food_y = 5

# Очки и длина
var score = 0
var length = 3
var speed = 50

# Игровой цикл (простая симуляция — змейка движется автоматически)
var frames = 0
var max_frames = 30

while frames < max_frames {
    # Рендер поля
    var y = 0
    while y < HEIGHT {
        var line = ""
        var x = 0
        while x < WIDTH {
            if x == head_x && y == head_y {
                line = line + "@"
            } else if x == food_x && y == food_y {
                line = line + "#"
            } else if y == 0 || y == HEIGHT - 1 {
                line = line + "-"
            } else if x == 0 || x == WIDTH - 1 {
                line = line + "|"
            } else {
                line = line + " "
            }
            x = x + 1
        }
        print line
        y = y + 1
    }

    print "Score: " + score + "  Length: " + length + "  Frame: " + frames
    print "Use WASD in shell to control!"

    # Движение змейки
    head_x = head_x + dir_x
    head_y = head_y + dir_y

    # Проверка столкновения со стеной
    if head_x <= 0 || head_x >= WIDTH - 1 {
        print "Game Over! Hit the wall!"
        beep 200 500
        sleep 100
        beep 0
        frames = max_frames
    }

    if head_y <= 0 || head_y >= HEIGHT - 1 {
        print "Game Over! Hit the wall!"
        beep 200 500
        sleep 100
        beep 0
        frames = max_frames
    }

    # Проверка еды
    if head_x == food_x && head_y == food_y {
        score = score + 10
        length = length + 1
        beep 880 100
        # Новая еда (случайная позиция — простой генератор)
        food_x = (frames * 7 + 3) % (WIDTH - 2) + 1
        food_y = (frames * 11 + 5) % (HEIGHT - 2) + 1
    }

    # Автоматическое движение (простая змейка)
    if frames % 3 == 0 {
        # Поворот вправо
        var temp = dir_x
        dir_x = dir_y
        dir_y = temp
    }

    frames = frames + 1
}

print ""
print "Final Score: " + score
print "Thanks for playing Vibra Snake!"
beep 523 100
sleep 100
beep 659 100
sleep 100
beep 784 100
sleep 100
beep 0
