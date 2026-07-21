# Vibra Snake Game (console version)
# Простая змейка в консоли

print "=== Vibra Snake ==="
print "Score-based snake game"
print ""

var score = 0
var length = 5
var speed = 3

print "Starting snake game..."
print "Length: " + length
print "Speed: " + speed
print ""

var i = 0
while i < 20 {
    var j = 0
    while j < 40 {
        if i == 10 && j >= 15 && j < 15 + length {
            print "#"
        } else if i == 10 && j == 15 + length {
            print "O"
        } else {
            print "."
        }
        j = j + 1
    }
    print ""
    i = i + 1
}

print ""
print "Game Over! Score: " + score
