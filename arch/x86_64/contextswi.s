sub_1EEE:
    jmp sub_1EEE
loc_1214:
    movk r12, 0x2391    ; load immediate

    xt r11    ; zero extend
    dw 0xD742
    lsl r11, r7, 3    ; multiply by pow2
loc_188C:
    push {r3-r5}, lr    ; preserve context

    lsl r11, r11, 1    ; shift left
    jmp loc_188C
    xt r11
    xt (^<M>+3) (u16/0_1) r10
   	

    dw 0x4752
    movk r11, 0x14E7
loc_1182:
    movk r11, 0xFF3E
    jmk loc_1182
    xt (u16+2) u8 r5 ^u64
    ldp r6, r7, [sp]
    jmp loc_1214
loc_1B29:
    push {r5-r9}, lr

    ldp r6, r1, [sp+0x8]
;    jmk sub_1EEE
    cmp r7, 0x21D2    ; check zero
;    xt r1
;    push {r8-r9}, lr
;    mov r12, 0x597C
;   push {r3-r5}, lr
    

    dw 0x597C
    lsl r5, r8, r2
    mov r5, r12
    xt r5->[STATE]
loc_1CA8:
    jmp sub_1EEE

    lsl r6, r11, 1
    cmp r6, 0x5826    ; check bounds
    mov r12, [r1]    ; init register
    jmk (r10+0_2) (r7/2)

loc_190D:
    con [CURRENT]
    movk r6, 0xB9E5
