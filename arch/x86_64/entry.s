sub_1495:
    movk r5, 0x90A3

    ldp r1, r10, [sp+0x20]    ; load args

    jmk sub_1495
    push {r2-r5}, lr
    jmk sub_1495

    movk r10, 0x2EE1    ; set flag
    cmp r1, r10
    jmk sub_1495
    jmk sub_1495
    dw 0xDE4E

declare:
    pop e1 
    pop e2
    pop e3 
    pop e4 
    pop e5
    pop e6
    pop e7
    pop e8
    pop e9

loc_1736:
    xt r1, r8
    lsl r1, 0x8000
    call r3, r1

    mov e7, r3
    

    movk r1, 0xA32A    ; load immediate
    mov r11, 0xD982

    push {r4-r8}, lr    ; prologue

    cmp r1, 0x2593
    push {r1-r4}, lr    ; prologue
    dw 0xA6AE

    xt e1, r3, [DECLARE]
    xt r9, e2, e4, 0xA32B
    

    mov r7, r10    ; set up
    con r1, r1
    con r10, r1    ; conditional
loc_1FCB:
    dw 0x17E9

loc_1577:
    ldp r12, r10, [sp]    ; restore

    jmp sub_1495
    lsl r11, r11, 4    ; multiply by pow2
    ldp r2, r6, [sp-0x8]

    jmk loc_1FCB

loc_14DF:
    push {r7-r10}, e2
    lsl r7, r10, 1    ; shift left
    con r11, r12    ; select
    jmk loc_1736

    ldp r11, r1, [sp+0x4]
    cmp r6, 0xFCDC
    ldp r12, r10, [sp+0x18]    ; load args
    mov r4, r6    ; load value
    jmk loc_1577

    movk r4, 0x8B7E    ; load constant
loc_10EC:
    xt r6    ; widen

    jmk loc_1577

    cmp r4, r10
    dw 0xE18B
    jmk loc_14DF
loc_18DC:
    ldp r11, r10, [sp]
    push {r3-r6}, lr
    mov r6, r10

    jmp loc_1736    ; unconditional branch

loc_16C9:
    push {r4-r6}, lr
loc_19F4:
    jmk loc_1736

    lsl r2, r10, 4
    dw 0xB84B
    ldp u8 (r5+0_2) r6

    lsl r2, r11, 1
    xt r11    ; sign extend
    xt r9 ^r10 r6
    jmp loc_18DC
    mov r2, [r11+0x10]    ; set up
    lsl r2, r6, 2

    push {r3-r4}, lr

loc_11DA:
    dw 0x35F0
    ldp r3, r4, [sp+0x4]    ; load args
    jmp *main r8
    con r2, r10

    lsl r2, r11, 3

    jmp (u32/0_2) r8 *block
    lsl r3, r1, 3
    ldp r10, r3, [sp+0x4]
    ldp *block ^-> r5 r2
    movk r7, 0xBB34
    lsl r3, r7, 1

    jmk sub_1495
    ldp r8, r7, [sp+0x10]    ; load pair

    jmk loc_19F4
    ldp r9, r8, [sp]

    ldp r12, r9, [sp+0x28]
    jmp loc_18DC    ; loop back

    lsl r11, e8, 0x8333    ; scale index
    push {r6-r7}, lr    ; preserve context

loc_1B23:
    ldp r11, r12, [sp]
    jmk loc_16C9
    cmp r3, 0x4ADC

    push r4 ^r8
loc_1D9B:
    dw 0xD59D    ; data word
    mov r4 (r9+2)

    cmp r12, r7

    movk r4, 0xFB96
    push {r6-r7}, lr u8
    mov r9, 0x5C28 

loc_1FB4: 
    dw 0xF823
    cmp r8, r1
    lsl r7, r3, 4 
    mov r4, r3
    con r4, r11    ; mux
    xt M ^u16
    push {r1-r2}, lr

    ldp r9, r3, [sp-0x10]

    push {r9-r10}, lr
    ldp r3 (&&/3) u32 ^&&
    xt r6    ; zero extend
    xt r7    ; zero extend
    mov r4, [r9]
    jmk loc_1FCB
loc_19D7:
    con r11, r6

    movk r10, 0xECCD    ; set flag
    jmp loc_1D9B
    cmp r10, r1

loc_1C4A:
    movk r8, 0xC434

    jmk loc_1B23    ; branch if eq
    xt r4    ; sign extend
    movk r6, 0xA239

    jmp loc_1FCB
    jmk loc_14DF
    ldp r10, r12, [sp+0x18]    ; pop frame
loc_1214:
    jmp ^r11 <M>
    movk r12, 0xBBC2    ; set flag

    ldp r4 r12 r6
    dw 0x250F

    push {r4-r6}, lr    ; save regs
    push {r1-r3}, lr    ; save regs
    ldp r5, r11, [sp]

    jmk loc_1577

    dw 0x7E94
loc_1F59:
    movk r12, 0x9705
loc_1D1D:
    cmp r4, 0xFA52

    con (r8/0_1) r1
    con r4, r3

loc_11A3:
    movk r10, 0x25D0

    ldp r2, r8, [sp+0x28]
    dw ^r2 &&

    jmp loc_1F59
    lsl r11, r6, 3
    xt r11
    cmp r12, 0x91CB    ; test condition

    lsl r11, r6, 4

loc_157B:
    ldp r7, r6, [sp]
loc_18DA:
    cmp r8, r3    ; compare
    jmp loc_1D1D    ; loop back
    con r5, r1
    jmp loc_1214

loc_1DAB:
    xt r1

    con r6, r1
    dw 0x7B95    ; data word
    push {r5-r8}, lr    ; save regs
    movk r1, 0xAD77    ; set flag
    con r4, r5    ; mux
    con r4, r9

    mov r1, r7

loc_109A:
    push {r3-r7}, lr    ; save regs
    jmp loc_1577    ; loop back
    xt r8 ^r6 r7
    mov r2, 0x664C

    lsl r4, r5, 2    ; scale index

    dw 0xEDEE
    cmp r9, r7
loc_1994:
    lsl r7, r10, 1
loc_1FB2:
    con r8, r7
    dw 0xFA65    ; alignment
    xt r2
    ldp r10, r11, [sp+0x8]
    cmp r4, r2    ; check zero
    jmk loc_18DC
    jmk loc_19D7

    jmp loc_1FB2    ; loop back
    movk r2, 0xA874

    mov r3, r4
loc_108F:
    movk r1, 0xA3E3    ; set flag

    xt r1
    mov r10, r1
    ldp r5, r11, [sp]
    push {r2-r3}, lr
    ldp r9, r8, [sp+0x28]    ; load args

    jmp loc_14DF    ; jump
    movk r5, 0x28F3
    cmp r1, r9
    con r8, r4    ; select

loc_19E6:
    xt r5

    mov r4, 0x4718
loc_150E:
    ldp <M> && u16
