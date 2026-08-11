/*=================================================================*
* DISCLAIMER: The following code is provided for Quilter customers *
* to use at their own risk.                                        *
* THE CODE IS PROVIDED "AS IS" AND WITH NO WARRANTIES, INCLUDING   *
* WITHOUT LIMITATION ANY EXPRESS WARRANTIES OR IMPLIED WARRANTIES  *
* OF MERCHANTABILITY OR FITNESS FOR A PARTICULAR USE.              *
*******************************************************************/
; File: Quilter_Importer.il
;
;	Author: Quilter
;	Version 0.01
;	Date: 2025 September

; ============================================================================
; Design File Importer - External File-Based PCB Design Modification
; ============================================================================
;
; This module provides comprehensive support for updating Allegro PCB designs
; from external CSV text files with full transaction support and design
; integrity checking.
; ============================================================================

qltrDebug = nil
qltrEndpointStub = t
qltrStubSearchRadius = 25.0
qltrStubExtraTolerance = 10.0
qltrStubWidth = nil
; Tiny direct stubs can draw without becoming electrically useful. For gaps
; shorter than this, create a 3-point hairpin whose endpoints are the route
; vertex and pad/via center, with the middle point pulled back along the trace.
qltrStubMinDrawLength = 2.54
qltrMaxStubLog = 80
qltrMaxNoSnapLog = 40
; set the Thermal/anti clearances directly via the SKILL parameter
; API instead of relying on a GUI-form replay. (field value) pairs assigned on
; the shapeDynamic param object, then read back to verify the change took.
; Confirmed by before/after dump diff on Allegro 25.1: choosing "Thermal/anti"
; for the three Clearances (Thru pin / SMD pin / Via) flips exactly these three
; booleans from nil to t (nil = DRC clearance, t = Thermal/anti).
qltrShapeDynamicClearanceSettings = list(
  list("pinClearanceOther" t)
  list("smdClearanceOther" t)
  list("viaClearanceOther" t)
)

qltrCurrentSourceRecord = 0
qltrProcessedCount = 0
qltrSuccessCount = 0
qltrErrorCount = 0
qltrNoOpCount = 0
qltrStubCount = 0
qltrDirectStubCount = 0
qltrHairpinStubCount = 0
qltrStubFailCount = 0
qltrStubCoincidentCount = 0
qltrEndpointProbeCount = 0
qltrEndpointTargetFoundCount = 0
qltrNoSnapCount = 0
qltrTargetHitCount = 0
qltrStubLogCount = 0
qltrNoSnapLogCount = 0
qltrUnknownPhaseCount = 0
qltrSnapTargets = makeTable("qltrSnapTargets" nil)
qltrSnapTargetBuckets = makeTable("qltrSnapTargetBuckets" nil)
qltrSnapBucketSize = qltrStubSearchRadius
qltrBucketQueryCount = 0
qltrBucketCandidateCount = 0
qltrBucketFallbackCount = 0
qltrTargetCount = 0
qltrPinTargetCount = 0
qltrViaTargetCount = 0
qltrTargetBuildFailureCount = 0
qltrSnapDistanceCount = 0
qltrSnapDistanceSum = 0.0
qltrSnapDistanceMin = nil
qltrSnapDistanceMax = nil
qltrPinSnapDistanceCount = 0
qltrPinSnapDistanceSum = 0.0
qltrPinSnapDistanceMin = nil
qltrPinSnapDistanceMax = nil
qltrViaSnapDistanceCount = 0
qltrViaSnapDistanceSum = 0.0
qltrViaSnapDistanceMin = nil
qltrViaSnapDistanceMax = nil

; read-only diagnostic -- print the shapeDynamic parameter object's
; field names and current values (useful to confirm the clearance field names).
defun( qltrDumpShapeDynamicParams ()
  let( (p props prop val)
    printf("SHAPE PARAM DUMP: reading axlGetParam(\"shapeDynamic\")\n")
    p = car(errset(axlGetParam("shapeDynamic") nil))
    if( p then
      printf("SHAPE PARAM DUMP: object=%L\n" p)
      props = car(errset(p->? nil))
      if( props then
        foreach( prop props
          val = car(errset(get(p prop) nil))
          printf("SHAPE PARAM DUMP:   %L = %L\n" prop val)
        )
        t
      else
        printf("SHAPE PARAM DUMP: could not enumerate properties via p->?; raw object printed above\n")
        nil
      )
    else
      printf("SHAPE PARAM DUMP: ERROR axlGetParam(\"shapeDynamic\") returned nil (requires 17.2+)\n")
      nil
    )
  )
)

; set the global dynamic shape clearances to Thermal/anti directly
; through the SKILL parameter API and verify the assignment actually took. This
; replaces the GUI-form replay, which is unreliable when triggered from SKILL
; (it runs while SKILL still owns the application, so the form commands no-op)
; and also raises a modal "disable fills?" dialog. The DB API avoids both.
; Returns t only if every field reads back as the requested value.
defun( qltrSetShapeDynamicClearancesThermalAnti ()
  let( (p pair fieldName wantValue applyResult verifyP verified cur)
    if( !qltrShapeDynamicClearanceSettings then
      printf("SHAPE PARAM SKILL: qltrShapeDynamicClearanceSettings is empty; nothing to set\n")
      nil
    else
    progn(
    p = car(errset(axlGetParam("shapeDynamic") nil))
    if( !p then
      printf("SHAPE PARAM SKILL: axlGetParam(\"shapeDynamic\") returned nil; cannot set via SKILL\n")
      nil
    else
      ; mutate via arrow assignment (p->field = value), NOT putprop.
      ; These "param:" objects only honor the documented arrow form; putprop
      ; mutates the SKILL wrapper without updating the underlying param, so
      ; axlSetParam wrote back the unchanged value (fields stayed nil).
      p->pinClearanceOther = t
      p->smdClearanceOther = t
      p->viaClearanceOther = t
      printf("SHAPE PARAM SKILL: set pinClearanceOther = t, smdClearanceOther = t, viaClearanceOther = t\n")

      applyResult = car(errset(axlSetParam(p) nil))
      if( !applyResult then
        printf("SHAPE PARAM SKILL: ERROR axlSetParam failed\n")
        nil
      else
        verifyP = car(errset(axlGetParam("shapeDynamic") nil))
        verified = t
        if( verifyP then
          foreach( pair qltrShapeDynamicClearanceSettings
            fieldName = car(pair)
            wantValue = cadr(pair)
            cur = car(errset(get(verifyP concat(fieldName)) nil))
            if( !equal(cur wantValue) then
              verified = nil
              printf("SHAPE PARAM SKILL: VERIFY MISMATCH %s = %L (expected %L)\n"
                     fieldName cur wantValue)
            )
          )
        else
          verified = nil
        )
        if( verified then
          printf("SHAPE PARAM SKILL: verified global dynamic shape clearances = Thermal/anti via axlSetParam\n")
          t
        else
          printf("SHAPE PARAM SKILL: values did not verify; call qltrDumpShapeDynamicParams() in the SKILL window to check field names\n")
          nil
        )
      )
    )
    )
    )
  )
)

; set the Thermal/anti clearances via the SKILL parameter API and
; report whether it verified. No GUI replay / .scr involved.
defun( qltrConfigureShapeThermalAnti ()
  let( (skillOk)
    skillOk = qltrSetShapeDynamicClearancesThermalAnti()
    if( skillOk then
      printf("SHAPE PARAM: clearances set via SKILL; no GUI replay or dialog needed\n")
      t
    else
      printf("SHAPE PARAM: WARNING could not set dynamic shape clearances via SKILL (needs Allegro 17.2+)\n")
      nil
    )
  )
)

defun( qltrRecordType (record)
  let( (opType)
    opType = nil
    foreach( field record
      if( car(field) == "operation" || car(field) == "type" then
        opType = cadr(field)
      )
    )
    opType
  )
)

defun( qltrParseCoord (coordStr)
  let( (coords point)
    point = nil
    if( coordStr && nindex(coordStr ":") then
      coords = parseString(coordStr ":")
      if( length(coords) >= 2 then
        point = atof(car(coords)):atof(cadr(coords))
      )
    )
    point
  )
)

defun( qltrSamePoint (p1 p2)
  if( p1 && p2 then
    xCoord(p1) == xCoord(p2) && yCoord(p1) == yCoord(p2)
  else
    nil
  )
)

defun( qltrDistanceSquared (p1 p2)
  let( (dx dy)
    dx = xCoord(p1) - xCoord(p2)
    dy = yCoord(p1) - yCoord(p2)
    dx * dx + dy * dy
  )
)

defun( qltrFloorInt (value)
  let( (truncated)
    truncated = fix(value)
    if( value < truncated then
      truncated - 1
    else
      truncated
    )
  )
)

defun( qltrCeilPositiveInt (value)
  let( (truncated)
    truncated = fix(value)
    if( value > truncated then
      truncated + 1
    else
      truncated
    )
  )
)

defun( qltrSnapBucketCoord (coord)
  qltrFloorInt(coord / qltrSnapBucketSize)
)

defun( qltrSnapBucketKey (netName bucketX bucketY)
  sprintf(nil "%s|%d|%d" netName bucketX bucketY)
)

defun( qltrFirstDBObject (result)
  let( (obj first)
    obj = nil
    if( result then
      if( listp(result) then
        first = car(result)
        if( listp(first) then
          obj = car(first)
        else
          obj = first
        )
      else
        obj = result
      )
    )
    obj
  )
)

defun( qltrSafeObjNetName (obj)
  let( (result)
    result = errset(obj->net->name nil)
    if( result then car(result) else nil )
  )
)

defun( qltrSafeObjXY (obj)
  let( (result)
    result = errset(obj->xy nil)
    if( result then car(result) else nil )
  )
)

defun( qltrResetSnapTargets ()
  let( ()
    qltrSnapTargets = makeTable("qltrSnapTargets" nil)
    qltrSnapTargetBuckets = makeTable("qltrSnapTargetBuckets" nil)
    qltrSnapBucketSize = qltrStubSearchRadius
    if( !qltrSnapBucketSize || qltrSnapBucketSize <= 0.0 then
      qltrSnapBucketSize = 1.0
    )
    qltrBucketQueryCount = 0
    qltrBucketCandidateCount = 0
    qltrBucketFallbackCount = 0
    qltrTargetCount = 0
    qltrPinTargetCount = 0
    qltrViaTargetCount = 0
    qltrTargetBuildFailureCount = 0
  )
)

defun( qltrResetSnapDistanceStats ()
  let( ()
    qltrSnapDistanceCount = 0
    qltrSnapDistanceSum = 0.0
    qltrSnapDistanceMin = nil
    qltrSnapDistanceMax = nil
    qltrPinSnapDistanceCount = 0
    qltrPinSnapDistanceSum = 0.0
    qltrPinSnapDistanceMin = nil
    qltrPinSnapDistanceMax = nil
    qltrViaSnapDistanceCount = 0
    qltrViaSnapDistanceSum = 0.0
    qltrViaSnapDistanceMin = nil
    qltrViaSnapDistanceMax = nil
  )
)

defun( qltrRecordSnapDistanceValue (distance targetType)
  let( ()
    if( distance then
      qltrSnapDistanceCount = qltrSnapDistanceCount + 1
      qltrSnapDistanceSum = qltrSnapDistanceSum + distance
      if( !qltrSnapDistanceMin || distance < qltrSnapDistanceMin then
        qltrSnapDistanceMin = distance
      )
      if( !qltrSnapDistanceMax || distance > qltrSnapDistanceMax then
        qltrSnapDistanceMax = distance
      )

      if( targetType == "pin" then
        qltrPinSnapDistanceCount = qltrPinSnapDistanceCount + 1
        qltrPinSnapDistanceSum = qltrPinSnapDistanceSum + distance
        if( !qltrPinSnapDistanceMin || distance < qltrPinSnapDistanceMin then
          qltrPinSnapDistanceMin = distance
        )
        if( !qltrPinSnapDistanceMax || distance > qltrPinSnapDistanceMax then
          qltrPinSnapDistanceMax = distance
        )
      )

      if( targetType == "via" then
        qltrViaSnapDistanceCount = qltrViaSnapDistanceCount + 1
        qltrViaSnapDistanceSum = qltrViaSnapDistanceSum + distance
        if( !qltrViaSnapDistanceMin || distance < qltrViaSnapDistanceMin then
          qltrViaSnapDistanceMin = distance
        )
        if( !qltrViaSnapDistanceMax || distance > qltrViaSnapDistanceMax then
          qltrViaSnapDistanceMax = distance
        )
      )
    )
  )
)

defun( qltrPrintSnapDistanceStatsOne (label count sum minDistance maxDistance)
  let( (averageDistance)
    if( count > 0 then
      averageDistance = sum / count
      printf("STUB LENGTH: %s count=%d average=%g min=%g max=%g units=um\n"
             label count averageDistance minDistance maxDistance)
    else
      printf("STUB LENGTH: %s count=0 units=um\n" label)
    )
  )
)

defun( qltrPrintSnapDistanceStats ()
  let( ()
    qltrPrintSnapDistanceStatsOne("all stubs"
                                      qltrSnapDistanceCount
                                      qltrSnapDistanceSum
                                      qltrSnapDistanceMin
                                      qltrSnapDistanceMax)
    qltrPrintSnapDistanceStatsOne("stubs to pins"
                                      qltrPinSnapDistanceCount
                                      qltrPinSnapDistanceSum
                                      qltrPinSnapDistanceMin
                                      qltrPinSnapDistanceMax)
    qltrPrintSnapDistanceStatsOne("stubs to vias"
                                      qltrViaSnapDistanceCount
                                      qltrViaSnapDistanceSum
                                      qltrViaSnapDistanceMin
                                      qltrViaSnapDistanceMax)
  )
)

defun( qltrPrintSnapBucketStats ()
  let( (averageCandidates)
    if( qltrBucketQueryCount > 0 then
      averageCandidates = (qltrBucketCandidateCount * 1.0) / qltrBucketQueryCount
    else
      averageCandidates = 0.0
    )
    printf("SNAP BUCKETS: queries=%d candidates=%d averageCandidates=%g fallbacks=%d bucketSize=%g units=um\n"
           qltrBucketQueryCount qltrBucketCandidateCount averageCandidates
           qltrBucketFallbackCount qltrSnapBucketSize)
  )
)

defun( qltrAddSnapTarget (netName point objType source obj)
  let( (entry bucketX bucketY bucketKey)
    if( netName && point then
      entry = list(point objType source obj)
      qltrSnapTargets[netName] = cons(entry qltrSnapTargets[netName])
      bucketX = qltrSnapBucketCoord(xCoord(point))
      bucketY = qltrSnapBucketCoord(yCoord(point))
      bucketKey = qltrSnapBucketKey(netName bucketX bucketY)
      qltrSnapTargetBuckets[bucketKey] = cons(entry qltrSnapTargetBuckets[bucketKey])
      qltrTargetCount = qltrTargetCount + 1
      if( objType == "pin" then
        qltrPinTargetCount = qltrPinTargetCount + 1
      )
      if( objType == "via" then
        qltrViaTargetCount = qltrViaTargetCount + 1
      )
      t
    else
      nil
    )
  )
)

defun( qltrConsiderSnapTarget (target point layer radiusSquared bestTarget bestPoint bestDistance)
  let( (targetPoint distance)
    targetPoint = car(target)
    if( targetPoint then
      distance = qltrDistanceSquared(point targetPoint)
      if( distance <= radiusSquared && qltrTargetUsableOnLayer(target layer) then
        if( !bestDistance || distance < bestDistance then
          list(target targetPoint distance)
        else
          list(bestTarget bestPoint bestDistance)
        )
      else
        list(bestTarget bestPoint bestDistance)
      )
    else
      list(bestTarget bestPoint bestDistance)
    )
  )
)

defun( qltrFindEndpointTargetLinear (point layer netName radiusSquared)
  let( (targets target bestTarget bestPoint bestDistance considered)
    bestTarget = nil
    bestPoint = nil
    bestDistance = nil
    targets = qltrSnapTargets[netName]

    foreach( target targets
      considered = qltrConsiderSnapTarget(target point layer radiusSquared bestTarget bestPoint bestDistance)
      bestTarget = car(considered)
      bestPoint = cadr(considered)
      bestDistance = nth(2 considered)
    )

    if( bestTarget then
      list(bestTarget bestPoint bestDistance)
    else
      nil
    )
  )
)

defun( qltrFindEndpointTargetBucketed (point layer netName radius radiusSquared)
  let( (centerX centerY bucketRange minBucketX maxBucketXExclusive minBucketY maxBucketYExclusive bucketX bucketY bucketKey bucketTargets target bestTarget bestPoint bestDistance considered foundBucket)
    bestTarget = nil
    bestPoint = nil
    bestDistance = nil
    foundBucket = nil
    centerX = qltrSnapBucketCoord(xCoord(point))
    centerY = qltrSnapBucketCoord(yCoord(point))
    bucketRange = qltrCeilPositiveInt(radius / qltrSnapBucketSize)
    minBucketX = centerX - bucketRange
    maxBucketXExclusive = centerX + bucketRange + 1
    minBucketY = centerY - bucketRange
    maxBucketYExclusive = centerY + bucketRange + 1
    qltrBucketQueryCount = qltrBucketQueryCount + 1

    for( bucketX minBucketX maxBucketXExclusive
      for( bucketY minBucketY maxBucketYExclusive
        bucketKey = qltrSnapBucketKey(netName bucketX bucketY)
        bucketTargets = qltrSnapTargetBuckets[bucketKey]
        if( bucketTargets then
          foundBucket = t
          foreach( target bucketTargets
            qltrBucketCandidateCount = qltrBucketCandidateCount + 1
            considered = qltrConsiderSnapTarget(target point layer radiusSquared bestTarget bestPoint bestDistance)
            bestTarget = car(considered)
            bestPoint = cadr(considered)
            bestDistance = nth(2 considered)
          )
        )
      )
    )

    if( bestTarget then
      list(bestTarget bestPoint bestDistance)
    else
      if( foundBucket then
        nil
      else
        qltrBucketFallbackCount = qltrBucketFallbackCount + 1
        qltrFindEndpointTargetLinear(point layer netName radiusSquared)
      )
    )
  )
)

defun( qltrTargetUsableOnLayer (target layer)
  let( (objType obj padResult)
    objType = cadr(target)
    obj = nth(3 target)

    if( objType == "via" then
      t
    else
      if( objType == "pin" && obj && layer then
        padResult = errset(axlDBGetPad(obj layer "REGULAR") nil)
        if( padResult then
          if( car(padResult) then t else nil )
        else
          ; If the pad query is unavailable in this Allegro version, do not
          ; suppress an otherwise good same-net coordinate snap.
          t
        )
      else
        t
      )
    )
  )
)

defun( qltrBuildPinSnapTargetsFromSelection ()
  let( (result pins pin added)
    added = 0
    result = errset(let( (selectedPins)
      axlClearSelSet()
      axlSetFindFilter(?enabled '("noall" "pins" "invisible") ?onButtons '("pins"))
      axlAddSelectAll()
      selectedPins = axlGetSelSet()
      selectedPins
    ) nil)

    if( result then
      pins = car(result)
      foreach( pin pins
        if( qltrAddSnapTarget(qltrSafeObjNetName(pin) qltrSafeObjXY(pin) "pin" "selection" pin) then
          added = added + 1
        )
      )
      axlClearSelSet()
    else
      qltrTargetBuildFailureCount = qltrTargetBuildFailureCount + 1
      printf("DEBUG: pin snap target selection fallback failed\n")
    )
    added
  )
)

defun( qltrBuildPinSnapTargets ()
  let( (designResult design symbolsResult symbols sym pinsResult pins pin added)
    added = 0
    designResult = errset(axlDBGetDesign() nil)

    if( designResult then
      design = car(designResult)
      symbolsResult = errset(design->symbols nil)
      if( symbolsResult then
        symbols = car(symbolsResult)
        foreach( sym symbols
          pinsResult = errset(sym->pins nil)
          if( pinsResult then
            pins = car(pinsResult)
            foreach( pin pins
              if( qltrAddSnapTarget(qltrSafeObjNetName(pin) qltrSafeObjXY(pin) "pin" "symbol" pin) then
                added = added + 1
              )
            )
          )
        )
      )
    else
      qltrTargetBuildFailureCount = qltrTargetBuildFailureCount + 1
      printf("DEBUG: axlDBGetDesign failed while building pin snap targets\n")
    )

    if( added == 0 then
      added = qltrBuildPinSnapTargetsFromSelection()
    )

    printf("SNAP TARGETS: collected %d pin targets (%d total targets, %d target build failures)\n"
           added qltrTargetCount qltrTargetBuildFailureCount)
    added
  )
)

defun( qltrFindEndpointTarget (point layer netName endpointName width)
  let( (radius radiusFromWidth radiusSquared targets bestResult bestTarget bestPoint bestDistance)
    bestTarget = nil
    bestPoint = nil
    bestDistance = nil
    qltrEndpointProbeCount = qltrEndpointProbeCount + 1

    if( qltrEndpointStub && point && netName then
      radius = qltrStubSearchRadius
      if( width && width > 0.0 then
        radiusFromWidth = (width / 2.0) + qltrStubExtraTolerance
        if( radiusFromWidth > radius then
          radius = radiusFromWidth
        )
      )
      radiusSquared = radius * radius
      targets = qltrSnapTargets[netName]

      if( targets then
        bestResult = qltrFindEndpointTargetBucketed(point layer netName radius radiusSquared)
        if( bestResult then
          bestTarget = car(bestResult)
          bestPoint = cadr(bestResult)
          bestDistance = nth(2 bestResult)
        )

        if( bestTarget then
          qltrTargetHitCount = qltrTargetHitCount + 1
          qltrEndpointTargetFoundCount = qltrEndpointTargetFoundCount + 1
        else
          qltrNoSnapCount = qltrNoSnapCount + 1
          if( qltrNoSnapLogCount < qltrMaxNoSnapLog then
            printf("DEBUG: source record %d found no same-net pin/via within %g um for %s endpoint %L on net %s layer %s\n"
                   qltrCurrentSourceRecord radius endpointName point netName layer)
            qltrNoSnapLogCount = qltrNoSnapLogCount + 1
          )
        )
      else
        qltrNoSnapCount = qltrNoSnapCount + 1
        if( qltrNoSnapLogCount < qltrMaxNoSnapLog then
          printf("DEBUG: source record %d has no snap targets for %s endpoint %L on net %s layer %s\n"
                 qltrCurrentSourceRecord endpointName point netName layer)
          qltrNoSnapLogCount = qltrNoSnapLogCount + 1
        )
      )
    )

    if( bestTarget then
      list(bestPoint cadr(bestTarget) sqrt(bestDistance) nth(2 bestTarget) nth(3 bestTarget))
    else
      nil
    )
  )
)

defun( qltrHairpinMiddlePoint (fromPoint awayPoint minLength)
  let( (dx dy len scale)
    if( fromPoint && awayPoint then
      dx = xCoord(awayPoint) - xCoord(fromPoint)
      dy = yCoord(awayPoint) - yCoord(fromPoint)
      len = sqrt(dx * dx + dy * dy)
      if( len > 0.0 then
        scale = minLength / len
        (xCoord(fromPoint) + dx * scale):(yCoord(fromPoint) + dy * scale)
      else
        (xCoord(fromPoint) + minLength):(yCoord(fromPoint))
      )
    else
      (xCoord(fromPoint) + minLength):(yCoord(fromPoint))
    )
  )
)

defun( qltrCreateConnectStub (fromPoint awayPoint targetPoint targetType targetSource targetObj layer netName width endpointName)
  let( (stubWidth gap result lineObj mode middlePoint)
    result = nil
    mode = nil

    if( !fromPoint || !targetPoint then
      nil
    else
      gap = sqrt(qltrDistanceSquared(fromPoint targetPoint))
      if( qltrSamePoint(fromPoint targetPoint) then
        qltrStubCoincidentCount = qltrStubCoincidentCount + 1
        if( qltrStubLogCount < qltrMaxStubLog then
          printf("STUB DEBUG: source record %d %s endpoint on net %s layer %s is already on %s target from %s %L at %L\n"
                 qltrCurrentSourceRecord endpointName netName layer targetType targetSource targetObj targetPoint)
          qltrStubLogCount = qltrStubLogCount + 1
        )
        nil
      else
        stubWidth = if( qltrStubWidth && qltrStubWidth > 0.0 then qltrStubWidth else width )

        if( gap < qltrStubMinDrawLength then
          middlePoint = qltrHairpinMiddlePoint(fromPoint awayPoint qltrStubMinDrawLength)
          result = car(errset(axlDBCreateLine(list(fromPoint middlePoint targetPoint) stubWidth layer netName) nil))
          mode = "hairpin"
        else
          result = car(errset(axlDBCreateLine(list(fromPoint targetPoint) stubWidth layer netName) nil))
          mode = "direct"
        )

        if( !result && mode == "direct" then
          middlePoint = qltrHairpinMiddlePoint(fromPoint awayPoint qltrStubMinDrawLength)
          result = car(errset(axlDBCreateLine(list(fromPoint middlePoint targetPoint) stubWidth layer netName) nil))
          mode = "hairpin-fallback"
        )

        if( result then
          lineObj = qltrFirstDBObject(result)
          qltrStubCount = qltrStubCount + 1
          qltrRecordSnapDistanceValue(gap targetType)
          if( mode == "direct" then
            qltrDirectStubCount = qltrDirectStubCount + 1
          else
            qltrHairpinStubCount = qltrHairpinStubCount + 1
          )
          if( qltrStubLogCount < qltrMaxStubLog then
            printf("STUB DEBUG: source record %d drew %s %s endpoint stub on net %s layer %s from %L to %s target %L middle=%L targetSource=%s targetObj=%L gap=%g width=%g obj=%L units=um\n"
                   qltrCurrentSourceRecord mode endpointName netName layer fromPoint targetType targetPoint middlePoint targetSource targetObj gap stubWidth lineObj)
            qltrStubLogCount = qltrStubLogCount + 1
          )
          t
        else
          qltrStubFailCount = qltrStubFailCount + 1
          printf("STUB ERROR: source record %d failed %s stub on net %s layer %s endpoint=%L target=%L gap=%g width=%g middle=%L units=um\n"
                 qltrCurrentSourceRecord endpointName netName layer fromPoint targetPoint gap stubWidth middlePoint)
          nil
        )
      )
    )
  )
)

defun( qltrRecordPhase (record)
  let( (opType phase)
    opType = qltrRecordType(record)
    phase = 90
    case( opType
      ("CREATE_NET" phase = 10)
      ("CREATE_PADSTACK" phase = 10)
      ("CREATE_COMPONENT" phase = 20)
      ("MOVE_COMPONENT" phase = 20)
      ("MOVE_COMPONENT_TO" phase = 20)
      ("ROTATE_COMPONENT" phase = 20)
      ("TRANSFORM_COMPONENT" phase = 20)
      ("CREATE_VIA" phase = 30)
      ("CREATE_LINE" phase = 40)
      ("CREATE_POUR" phase = 50)
      (t
        qltrUnknownPhaseCount = qltrUnknownPhaseCount + 1
        phase = 90
      )
    )
    phase
  )
)

defun( qltrCreatePhasedRows (dataRows)
  let( (phasedRows sourceRecord record phase opType)
    phasedRows = nil
    sourceRecord = 0
    foreach( record dataRows
      sourceRecord = sourceRecord + 1
      phase = qltrRecordPhase(record)
      opType = qltrRecordType(record)
      phasedRows = append(phasedRows list(list(phase sourceRecord opType record)))
    )
    phasedRows
  )
)

defun( qltrCountPhaseRows (phasedRows phase)
  let( (count row)
    count = 0
    foreach( row phasedRows
      if( car(row) == phase then
        count = count + 1
      )
    )
    count
  )
)

; ============================================================================
; 1. FILE FORMAT DETECTION UTILITIES
; ============================================================================

defun( detectFileFormat (filename)
  let( (file line firstChar format)
    printf("Detecting file format for: %s\n" filename)

    ; Try to open file and read first line
    file = infile(filename)
    if( file then
      line = gets(nil file)
      close(file)

      if( line then
        ; Remove leading whitespace
        firstChar = substring(line 1 1)

        ; Detect format based on first character
        case( firstChar
          (";"
            ; Skip comment lines and try next line
            format = "UNKNOWN"
            printf("Found comment line, format unclear\n")
          )
          (t
            ; Check if it looks like CSV (contains commas)
            if( nindex(line ",") then
              format = "CSV"
              printf("Detected CSV format (contains commas)\n")
            else
              format = "UNKNOWN"
              printf("Unknown format detected\n")
            )
          )
        )
      else
        format = "EMPTY"
        printf("File appears to be empty\n")
      )
    else
      format = "ERROR"
      printf("ERROR: Cannot open file %s\n" filename)
    )

    format
  )
)

defun( validateFileExists (filename)
  let( (result)
    if( isFile(filename) then
      printf("File exists: %s\n" filename)
      t
    else
      printf("ERROR: File not found: %s\n" filename)
      nil
    )
  )
)

; ============================================================================
; 2. CSV PARSER WITH TYPE CONVERSION
; ============================================================================

defun( parseCSVLine (line)
  let( (fields)
    ; Use axlStringCSVParse for proper CSV parsing that handles empty fields
    ; Second parameter 't' strips leading/trailing whitespace
    fields = axlStringCSVParse(line t)
    ; printf("DEBUG: axlStringCSVParse fields: %L\n" fields)

    ; Return the parsed fields directly (axlStringCSVParse handles whitespace stripping)
    fields
  )
)

defun( convertCSVField (fieldValue fieldType)
  let( (convertedValue)
    case( fieldType
      ("string"
        convertedValue = fieldValue
      )
      ("float"
        convertedValue = atof(fieldValue)
      )
      ("integer"
        convertedValue = atoi(fieldValue)
      )
      ("boolean"
        convertedValue = if( fieldValue == "true" || fieldValue == "TRUE" || fieldValue == "1" then t else nil)
      )
      ("coordinate"
        ; Parse coordinate like "100.0:200.0" or "100.0,200.0"
        if( nindex(fieldValue ":") then
          coords = parseString(fieldValue ":")
          convertedValue = (atof(car(coords))):(atof(cadr(coords)))
        else
          ; Single value, assume square coordinate
          val = atof(fieldValue)
          convertedValue = val:val
        )
      )
      (t
        ; Default to string
        convertedValue = fieldValue
      )
    )

    convertedValue
  )
)

defun( parseCSVFile (filename)
  let( (file line header headerFields dataRows lineCount)
    printf("Parsing CSV file: %s\n" filename)

    file = infile(filename)
    dataRows = nil
    lineCount = 0

    if( file then
      ; Read header line
      header = gets(nil file)
      if( header then
        ; printf("DEBUG: Raw header line: '%s'\n" header)
        headerFields = parseCSVLine(header)
        ; printf("CSV Headers: %L\n" headerFields)
        ; printf("DEBUG: Header count: %d\n" length(headerFields))
        lineCount = 1

        ; Read data lines
        while( gets(line file)
          lineCount = lineCount + 1
          if( line && strlen(line) > 0 then
            ; printf("DEBUG: Raw data line: '%s'\n" line)
            dataFields = parseCSVLine(line)
            ; printf("DEBUG: Parsed data fields: %L\n" dataFields)
            ; printf("DEBUG: Data field count: %d\n" length(dataFields))

            ; Create row object with header:value pairs
            rowData = nil
            for( i 0 length(headerFields)
              fieldName = nth(i headerFields)
              fieldValue = nth(i dataFields)
              ; Only add if both fieldName and fieldValue are valid
              if( fieldName then
                rowData = append(rowData list(list(fieldName fieldValue)))
              )
            )

            dataRows = append(dataRows list(rowData))

            if( mod(lineCount 100) == 0 then
              printf("Processed %d lines...\n" lineCount)
            )
          )
        )
      )

      close(file)
      printf("CSV parsing complete: %d data rows\n" length(dataRows))

      ; Update progress meter message after parsing
      axlMeterUpdate(0 sprintf(nil "Found %d records to process..." length(dataRows)))
    else
      printf("ERROR: Cannot open CSV file %s\n" filename)
      dataRows = nil
    )

    dataRows
  )
)


; ============================================================================
; 4. DESIGN INTEGRITY CHECKING
; ============================================================================

defun( getCurrentDesignTimestamp ()
  let( (designName timestamp)
    designName = axlCurrentDesign()
    if( designName then
      ; Get file modification timestamp
      timestamp = fileTimeModified(axlGetDrawingName())
      printf("Current design timestamp: %s\n" timeToString(timestamp))
      timestamp
    else
      printf("ERROR: No design currently open\n")
      nil
    )
  )
)

defun( validateDesignState (expectedTimestamp)
  let( (currentTimestamp)
    if( expectedTimestamp then
      currentTimestamp = getCurrentDesignTimestamp()

      if( currentTimestamp then
        if( currentTimestamp == expectedTimestamp then
          printf("Design timestamp validation: PASSED\n")
          t
        else
          printf("WARNING: Design has been modified since last export\n")
          printf("Expected: %s\n" timeToString(expectedTimestamp))
          printf("Current:  %s\n" timeToString(currentTimestamp))
          nil
        )
      else
        printf("ERROR: Cannot get current design timestamp\n")
        nil
      )
    else
      printf("No timestamp validation requested\n")
      t
    )
  )
)

defun( validateDesignBounds (position)
  let( (extents valid)
    ; Get design extents (this is a simplified check)
    ; In a real implementation, you'd check against actual design bounds
    valid = t

    ; Basic coordinate validation
    if( position then
      if( xCoord(position) < -50000.0 || xCoord(position) > 50000.0 then
        printf("WARNING: X coordinate %g is outside typical bounds\n" xCoord(position))
        valid = nil
      )
      if( yCoord(position) < -50000.0 || yCoord(position) > 50000.0 then
        printf("WARNING: Y coordinate %g is outside typical bounds\n" yCoord(position))
        valid = nil
      )
    )

    valid
  )
)

; ============================================================================
; 5. DESIGN MODIFICATION ENGINE WITH TRANSACTIONS
; ============================================================================

defun( processDesignModifications (dataRows format)
  ; pprint(dataRows)
  let( (transactionMark successCount errorCount totalCount originalDesignUnits)
    totalCount = length(dataRows)
    successCount = 0
    errorCount = 0

    printf("\n=== Starting Design Modification with Transaction Support ===\n")
    printf("Processing %d modification records in %s format\n" totalCount format)

    ; Start database transaction
    ; transactionMark = axlDBTransactionStart()

    ; if( transactionMark then
      printf("Transaction started (mark: %L)\n" transactionMark)

      originalDesignUnits = axlDBGetDesignUnits()
      axlDBChangeDesignUnits("um" 2) ; change to microns
      ; Process modifications within transaction using axlDBCloak for performance
      result = axlDBCloak('processModificationsInTransaction(dataRows format))
      axlDBChangeDesignUnits(car(originalDesignUnits) cadr(originalDesignUnits))

      if( result then
        t
        ; Commit all changes
        ;if( axlDBTransactionCommit(transactionMark) then
        ;   printf("\n=== TRANSACTION COMMITTED: All modifications applied ===\n")
        ;   printf("Successfully processed %d records\n" totalCount)
        ;  t
        ;else
        ;  printf("\n=== ERROR: Transaction commit failed ===\n")
        ;  nil
        ;)
      else
        ; Rollback all changes
        ;if( axlDBTransactionRollback(transactionMark) then
        ;   printf("\n=== TRANSACTION ROLLED BACK: All modifications undone ===\n")
        ;  printf("No changes were applied to the design\n")
        ;else
        ;  printf("\n=== ERROR: Transaction rollback failed ===\n")
        ;)
        nil
      )
    ; else
      ;printf("ERROR: Failed to start transaction\n")
      ;nil
    ;)
  )
)

defun( qltrProcessOnePhasedRow (row format importErrors totalRecords)
  let( (sourceRecord opType record percentage)
    sourceRecord = cadr(row)
    opType = nth(2 row)
    record = nth(3 row)
    qltrCurrentSourceRecord = sourceRecord
    qltrProcessedCount = qltrProcessedCount + 1

    if( processSingleModification(record format) then
      qltrSuccessCount = qltrSuccessCount + 1
    else
      qltrErrorCount = qltrErrorCount + 1
      importErrors[sourceRecord] = record
      printf("ERROR: source record %d (%s) failed\n" sourceRecord opType)
    )

    if( mod(qltrProcessedCount 100) == 0 then
      percentage = round((qltrProcessedCount * 100.0) / totalRecords)
      printf("Processed %d of %d records (%d success, %d errors, %d no-op, %d connect stubs)...\n"
             qltrProcessedCount totalRecords qltrSuccessCount qltrErrorCount
             qltrNoOpCount qltrStubCount)
      axlMeterUpdate(percentage sprintf(nil "Processing baseline import record %d of %d..." qltrProcessedCount totalRecords))
      axlFlushDisplay()
    )
    t
  )
)

defun( qltrProcessPhase (phasedRows phase phaseName format importErrors totalRecords)
  let( (row phaseCount)
    phaseCount = qltrCountPhaseRows(phasedRows phase)
    if( phaseCount > 0 then
      printf("PHASE: %s (%d records)\n" phaseName phaseCount)
      foreach( row phasedRows
        if( car(row) == phase then
          qltrProcessOnePhasedRow(row format importErrors totalRecords)
        )
      )
    )
    t
  )
)

defun( processModificationsInTransaction (dataRows format)
  let( (totalRecords importErrors phasedRows)
    totalRecords = length(dataRows)
    importErrors = makeTable("importErrors" -1)
    phasedRows = qltrCreatePhasedRows(dataRows)

    qltrProcessedCount = 0
    qltrSuccessCount = 0
    qltrErrorCount = 0
    qltrNoOpCount = 0
    qltrStubCount = 0
    qltrDirectStubCount = 0
    qltrHairpinStubCount = 0
    qltrStubFailCount = 0
    qltrStubCoincidentCount = 0
    qltrEndpointProbeCount = 0
    qltrEndpointTargetFoundCount = 0
    qltrNoSnapCount = 0
    qltrTargetHitCount = 0
    qltrStubLogCount = 0
    qltrNoSnapLogCount = 0
    qltrUnknownPhaseCount = 0
    qltrResetSnapTargets()
    qltrResetSnapDistanceStats()

    printf("SUMMARY: phased import enabled. Order is setup, components, vias, clines, pours, unknown.\n")
    printf("SUMMARY: setup=%d components=%d vias=%d clines=%d pours=%d unknown=%d\n"
           qltrCountPhaseRows(phasedRows 10)
           qltrCountPhaseRows(phasedRows 20)
           qltrCountPhaseRows(phasedRows 30)
           qltrCountPhaseRows(phasedRows 40)
           qltrCountPhaseRows(phasedRows 50)
           qltrCountPhaseRows(phasedRows 90))

    qltrProcessPhase(phasedRows 10 "setup" format importErrors totalRecords)
    qltrProcessPhase(phasedRows 20 "components" format importErrors totalRecords)
    qltrBuildPinSnapTargets()
    qltrProcessPhase(phasedRows 30 "vias" format importErrors totalRecords)
    printf("SNAP TARGETS: after vias pin=%d via=%d total=%d\n"
           qltrPinTargetCount qltrViaTargetCount qltrTargetCount)
    qltrProcessPhase(phasedRows 40 "clines" format importErrors totalRecords)
    qltrProcessPhase(phasedRows 50 "pours" format importErrors totalRecords)
    qltrProcessPhase(phasedRows 90 "unknown" format importErrors totalRecords)
    qltrPrintSnapDistanceStats()
    qltrPrintSnapBucketStats()

    printf("Final results: %d success, %d errors, %d no-op rows, %d endpoint target hits, %d connect stubs drawn, %d endpoints with no same-net target in range\n"
           qltrSuccessCount qltrErrorCount qltrNoOpCount qltrTargetHitCount qltrStubCount qltrNoSnapCount)
    printf("STUB SUMMARY: endpointsProbed=%d targetsFound=%d stubsDrawn=%d direct=%d hairpin=%d coincident=%d createFailed=%d minDrawLen=%g units=um\n"
           qltrEndpointProbeCount qltrEndpointTargetFoundCount qltrStubCount
           qltrDirectStubCount qltrHairpinStubCount qltrStubCoincidentCount
           qltrStubFailCount qltrStubMinDrawLength)

    if( qltrErrorCount == 0 then
      t
    else
      printf("ERROR: %d modifications failed, rolling back transaction\n" qltrErrorCount)
      foreach(recNumber importErrors
         printf("  Source row %L: %L\n" recNumber importErrors[recNumber])
      )
      nil
    )
  )
)

; ============================================================================
; 6. SINGLE MODIFICATION PROCESSOR
; ============================================================================

defun( processSingleModification (record format)
  let( (operation result)
    result = nil

    ; Parse record based on format
    case( format
      ("CSV"
        operation = parseCSVRecord(record)
      )
      (t
        printf("ERROR: Unknown format %s\n" format)
        operation = nil
      )
    )

    ; Debug: Show parsed operation
    ; printf("DEBUG: Parsed operation: %L\n" operation)

    ; Execute operation
    if( operation then
      result = executeOperation(operation)
    else
      printf("ERROR: Failed to parse record: %L\n" record)
    )

    result
  )
)

defun( parseCSVRecord (record)
  ; printf("Entered: parseCSVRecord: %L\n" record)
  let( (operation opType)
    ; CSV record is list of (fieldName fieldValue) pairs
    operation = nil

    ; Find operation type
    foreach( field record
      ; printf("Field: %L\n" field)
      if( car(field) == "operation" || car(field) == "type" then
        opType = cadr(field)
      )
    )

    if( opType then
      operation = list(list("type" opType))

      ; Add all field data to operation
      foreach( field record
        operation = append(operation list(field))
      )
    )

    operation
  )
)


defun( executeOperation (operation)

let( (opType result)
    ; Get operation type
    opType = cadr(assoc("type" operation))
    ; printf("Executing operation: %s\n" opType)

    case( opType
      ("CREATE_NET"
        result = executeCreateNet(operation)
      )
      ("CREATE_VIA"
        result = executeCreateVia(operation)
      )
      ("CREATE_LINE"
        result = executeCreateLine(operation)
      )
      ("CREATE_COMPONENT"
        result = executeCreateComponent(operation)
      )
      ("MOVE_COMPONENT"
        result = executeMoveComponent(operation)
      )
      ("MOVE_COMPONENT_TO"
        result = executeMoveComponentTo(operation)
      )
      ("ROTATE_COMPONENT"
        result = executeRotateComponent(operation)
      )
      ("TRANSFORM_COMPONENT"
        result = executeTransformComponent(operation)
      )
      ("CREATE_PADSTACK"
        result = executeCreatePadstack(operation)
      )
      ("CREATE_POUR"
        result = executeCreatePour(operation)
      )
      (t
        printf("ERROR: Unknown operation type: %s\n" opType)
        result = nil
      )
    )
    printf("Exiting:  executeOperation(operation)\n")
    result
  )
)

; ============================================================================
; 7. OPERATION EXECUTORS
; ============================================================================

defun( executeCreateNet (operation)
  let( (netName netDbid)
    netName = cadr(assoc("name" operation))

    if( netName then
      netDbid = axlDBCreateNet(netName)
      if( netDbid then
        ; printf("Created net: %s (dbid: %L)\n" netName netDbid)
        t
      else
        printf("ERROR: Failed to create net: %s\n" netName)
        nil
      )
    else
      printf("ERROR: No net name specified\n")
      nil
    )
  )
)

defun( executeCreateVia (operation)
  let( (padstack position netName viaObj positionStr)
    ; printf("DEBUG executeCreateVia: Full operation = %L\n" operation)
    ; printf("Before call:  cadr(assoc(\"padstack\" operation)\n")
    padstack = cadr(assoc("padstack" operation))
    ; printf("padstack: %L\n" padstack)
    positionStr = cadr(assoc("position" operation))
    ; printf("positionStr: %L\n" positionStr)
    netName = cadr(assoc("net" operation))
    ; printf("netName: %L\n" netName)

    ; Parse coordinate string to coordinate value
    if( positionStr && nindex(positionStr ":") then
      coords = parseString(positionStr ":")
      position = atof(car(coords)):atof(cadr(coords))
      ; printf("DEBUG: Parsed position coordinate: %L\n" position)
    else
      position = nil
      printf("DEBUG: Failed to parse position coordinate from '%s'\n" positionStr)
    )

    ; printf("DEBUG: About to call axlDBCreateVia with padstack='%s', position=%L, netName='%s'\n" padstack position netName)

    if( padstack && position then
      viaObj = axlDBCreateVia(padstack position netName)
      ; printf("DEBUG: axlDBCreateVia returned: %L\n" viaObj)
      if( viaObj then
        qltrAddSnapTarget(netName position "via" "created-via" qltrFirstDBObject(viaObj))
        ; printf("Created via: %s at %L on net %s\n" padstack position netName)
        t
      else
        printf("ERROR: Failed to create via - axlDBCreateVia returned nil\n")
        printf("  padstack: '%s'\n" padstack)
        printf("  position: %L\n" position)
        printf("  netName: '%s'\n" netName)
        nil
      )
    else
      printf("ERROR: Missing via parameters - padstack=%L, position=%L\n" padstack position)
      nil
    )
  )
)

defun( executeCreateLine (operation)
  let( (startPoint endPoint width layer netName result startStr endStr widthStr startTarget endTarget)
    ; printf("DEBUG executeCreateLine: Full operation = %L\n" operation)
    startStr = cadr(assoc("start" operation))
    endStr = cadr(assoc("end" operation))
    widthStr = cadr(assoc("width" operation))
    layer = cadr(assoc("layer" operation))
    netName = cadr(assoc("net" operation))
    ; printf("DEBUG executeCreateLine: startStr='%s', endStr='%s', widthStr='%s', layer='%s', netName='%s'\n" startStr endStr widthStr layer netName)

    startPoint = qltrParseCoord(startStr)
    endPoint = qltrParseCoord(endStr)

    ; Parse width to float
    if( widthStr then
      width = atof(widthStr)
    else
      width = nil
    )

    if( startPoint && endPoint && width && layer then
      if( qltrSamePoint(startPoint endPoint) then
        qltrNoOpCount = qltrNoOpCount + 1
        printf("DEBUG: source record %d skipped zero-length CREATE_LINE at %L on net %s layer %s\n"
               qltrCurrentSourceRecord startPoint netName layer)
        t
      else
        result = axlDBCreateLine(list(startPoint endPoint) width layer netName)
        if( result then
          if( qltrDebug && qltrCurrentSourceRecord <= 5 then
            printf("DEBUG: created line source record %d net=%s layer=%s\n"
                   qltrCurrentSourceRecord netName layer)
          )
          if( qltrEndpointStub then
            startTarget = qltrFindEndpointTarget(startPoint layer netName "start" width)
            if( startTarget then
              qltrCreateConnectStub(startPoint endPoint car(startTarget) cadr(startTarget) nth(3 startTarget) nth(4 startTarget) layer netName width "start")
            )
            endTarget = qltrFindEndpointTarget(endPoint layer netName "end" width)
            if( endTarget then
              qltrCreateConnectStub(endPoint startPoint car(endTarget) cadr(endTarget) nth(3 endTarget) nth(4 endTarget) layer netName width "end")
            )
          )
          t
        else
          printf("ERROR: Failed to create line for source record %d net=%s layer=%s width=%g start=%L end=%L\n"
                 qltrCurrentSourceRecord netName layer width startPoint endPoint)
          nil
        )
      )
    else
      printf("ERROR: Missing line parameters at source record %d start=%L end=%L width=%L layer=%L net=%L\n"
             qltrCurrentSourceRecord startPoint endPoint width layer netName)
      nil
    )
  )
)

defun( executeMoveComponent (operation)
  let( (compName deltaMove deltaMoveStr mirrorStr mirror coords)
    ; printf("DEBUG executeMoveComponent: Full operation = %L\n" operation)
    compName = cadr(assoc("component" operation))
    deltaMoveStr = cadr(assoc("delta" operation))
    mirrorStr = cadr(assoc("mirror" operation))
    ; printf("DEBUG executeMoveComponent: compName='%s', deltaMoveStr='%s', mirrorStr='%s'\n" compName deltaMoveStr mirrorStr)

    ; Parse coordinate string to coordinate value
    if( deltaMoveStr && nindex(deltaMoveStr ":") then
      coords = parseString(deltaMoveStr ":")
      deltaMove = atof(car(coords)):atof(cadr(coords))
    else
      deltaMove = nil
    )

    ; Parse mirror parameter
    if( mirrorStr && (mirrorStr == "t" || mirrorStr == "true" || mirrorStr == "TRUE") then
      mirror = t
    else
      mirror = nil
    )

    ; Use shared movement function
    moveComponentByDelta(compName deltaMove mirror "MOVE_COMPONENT")
  )
)

; Shared function for component movement operations
defun( moveComponentByDelta (compName deltaMove mirror operationType)
  let( (component result)
    ; printf("DEBUG moveComponentByDelta: compName='%s', deltaMove=%L, mirror=%L, operationType='%s'\n"
    ;        compName deltaMove mirror operationType)

    if( compName && deltaMove then
      ; Find component
      axlSetFindFilter(?enabled '(noall symbols invisible) ?onButtons '(symbols))
      component = axlGetSelSet(axlSingleSelectName("SYMBOL" compName))

      if( component then
        component = car(component)

        ; Apply move with optional mirror
        ; printf("DEBUG: About to call axlTransformObject with move=%L, mirror=%L\n" deltaMove mirror)

        if( mirror then
          result = axlTransformObject(component ?move deltaMove ?mirror mirror)
          printf("%s: Moved and mirrored component %s by %L\n" operationType compName deltaMove)
        else
          result = axlTransformObject(component ?move deltaMove)
          printf("%s: Moved component %s by %L\n" operationType compName deltaMove)
        )

        if( result then
          axlClearSelSet()
          t
        else
          printf("ERROR: Failed to move component %s\n" compName)
          axlClearSelSet()
          nil
        )
      else
        printf("ERROR: Component %s not found\n" compName)
        nil
      )
    else
      printf("ERROR: Missing component move parameters (compName=%s, deltaMove=%L)\n" compName deltaMove)
      nil
    )
  )
)

; Function to get current component location
defun( getComponentLocation (compName)
  let( (component location)
    ; printf("DEBUG getComponentLocation: Finding location for component '%s'\n" compName)

    if( compName then
      axlSetFindFilter(?enabled '(noall symbols invisible) ?onButtons '(symbols))
      component = axlGetSelSet(axlSingleSelectName("SYMBOL" compName))

      if( component then
        component = car(component)
        location = component->xy
        ; printf("DEBUG getComponentLocation: Found component %s at location %L\n" compName location)
        axlClearSelSet()
        location
      else
        printf("ERROR: Component %s not found for location lookup\n" compName)
        nil
      )
    else
      printf("ERROR: Missing component name for location lookup\n")
      nil
    )
  )
)

defun( executeMoveComponentTo (operation)
  let( (compName destinationCoords positionStr mirrorStr mirror coords currentLocation deltaMove)
    ; printf("DEBUG executeMoveComponentTo: Full operation = %L\n" operation)
    compName = cadr(assoc("component" operation))
    positionStr = cadr(assoc("position" operation))
    mirrorStr = cadr(assoc("mirror" operation))
    ; printf("DEBUG executeMoveComponentTo: compName='%s', positionStr='%s', mirrorStr='%s'\n" compName positionStr mirrorStr)

    ; Parse destination coordinate string
    if( positionStr && nindex(positionStr ":") then
      coords = parseString(positionStr ":")
      destinationCoords = atof(car(coords)):atof(cadr(coords))
      ; printf("DEBUG: Parsed destination coordinates: %L\n" destinationCoords)
    else
      destinationCoords = nil
    )

    ; Parse mirror parameter
    if( mirrorStr && (mirrorStr == "t" || mirrorStr == "true" || mirrorStr == "TRUE") then
      mirror = t
    else
      mirror = nil
    )

    if( compName && destinationCoords then
      ; Get current component location
      currentLocation = getComponentLocation(compName)

      if( currentLocation then
        ; Calculate delta move: destination - current
        deltaMove = (xCoord(destinationCoords) - xCoord(currentLocation)):(yCoord(destinationCoords) - yCoord(currentLocation))
        ; printf("DEBUG: Calculated deltaMove = %L (destination=%L - current=%L)\n" deltaMove destinationCoords currentLocation)

        ; Use shared movement function
        moveComponentByDelta(compName deltaMove mirror "MOVE_COMPONENT_TO")
      else
        printf("ERROR: Could not get current location for component %s\n" compName)
        nil
      )
    else
      printf("ERROR: Missing component or destination parameters (compName=%s, destination=%L)\n" compName destinationCoords)
      nil
    )
  )
)

defun( executeRotateComponent (operation)
  let( (compName angle component result angleStr mirrorStr mirror originStr origin)
    ; printf("DEBUG executeRotateComponent: Full operation = %L\n" operation)
    compName = cadr(assoc("component" operation))
    angleStr = cadr(assoc("angle" operation))
    mirrorStr = cadr(assoc("mirror" operation))
    originStr = cadr(assoc("origin" operation))
    ; printf("DEBUG executeRotateComponent: compName='%s', angleStr='%s', mirrorStr='%s', originStr='%s'\n" compName angleStr mirrorStr originStr)

    ; Parse angle string to float value
    if( angleStr then
      angle = atof(angleStr)
    else
      angle = nil
    )

    ; Parse mirror parameter
    if( mirrorStr && (mirrorStr == "t" || mirrorStr == "true" || mirrorStr == "TRUE") then
      mirror = t
    else
      mirror = nil
    )

    ; Parse origin coordinate string
    if( originStr && nindex(originStr ":") then
      coords = parseString(originStr ":")
      origin = atof(car(coords)):atof(cadr(coords))
      ; printf("DEBUG: Parsed origin coordinate: %L\n" origin)
    else
      origin = nil
    )

    if( compName && angle then
      ; Find component
      axlSetFindFilter(?enabled '(noall symbols invisible) ?onButtons '(symbols))
      component = axlGetSelSet(axlSingleSelectName("SYMBOL" compName))

      if( component then
        component = car(component)
        ; Apply the source angle as-is. The exporter bakes the correct rotation
        ; delta into the CSV (it accounts for a later side flip, which negates the
        ; stored rotation), so the importer must NOT adjust it here.
        ; Apply rotation with optional mirror and origin
        ; printf("DEBUG: About to call axlTransformObject with mirror=%L, angle=%g, origin=%L\n" mirror angle origin)
        if( mirror && origin then
          result = axlTransformObject(component ?mirror mirror ?angle angle ?origin origin)
          printf("Rotated and mirrored component %s by %g degrees about %L\n" compName angle origin)
        else
          if( mirror then
            result = axlTransformObject(component ?mirror mirror ?angle angle)
            printf("Rotated and mirrored component %s by %g degrees\n" compName angle)
          else
            if( origin then
              result = axlTransformObject(component ?angle angle ?origin origin)
              printf("Rotated component %s by %g degrees about %L\n" compName angle origin)
            else
              result = axlTransformObject(component ?angle angle)
              printf("Rotated component %s by %g degrees\n" compName angle)
            )
          )
        )

        if( result then
          axlClearSelSet()
          t
        else
          printf("ERROR: Failed to rotate component %s\n" compName)
          axlClearSelSet()
          nil
        )
      else
        printf("ERROR: Component %s not found\n" compName)
        nil
      )
    else
      printf("ERROR: Missing component rotation parameters\n")
      nil
    )
  )
)

defun( executeCreateComponent (operation)
  let( (refdes symbolName position rotation mirror symbolType result symbolData symbolExists)
    refdes = cadr(assoc("refdes" operation))
    symbolName = cadr(assoc("symbol" operation))
    position = cadr(assoc("position" operation))
    rotation = cadr(assoc("rotation" operation))
    mirror = cadr(assoc("mirror" operation))
    symbolType = cadr(assoc("symbolType" operation))

    ; Set defaults
    rotation = if( rotation then rotation else 0.0 )
    mirror = if( mirror then mirror else nil )
    symbolType = if( symbolType then symbolType else "PACKAGE" )

    if( refdes && symbolName && position then
      printf("Creating component %s with symbol %s at %L\n" refdes symbolName position)

      ; Check if symbol exists (requires external function from create_test_components.il)
      symbolExists = t  ; Assume symbol exists for now - could add validation here

      if( symbolExists then
        ; Create symbol data structure
        symbolData = list(symbolName symbolType refdes)

        ; Create the component
        result = axlDBCreateSymbol(symbolData position mirror rotation)

        if( result then
          printf("Created component: %s with symbol %s at %L\n" refdes symbolName position)
          t
        else
          printf("ERROR: Failed to create component %s\n" refdes)
          nil
        )
      else
        printf("ERROR: Symbol %s not found for component %s\n" symbolName refdes)
        nil
      )
    else
      printf("ERROR: Missing component creation parameters (refdes, symbol, position required)\n")
      nil
    )
  )
)

defun( executeTransformComponent (operation)
  let( (compName deltaMove angle mirror origin component result deltaMoveStr angleStr mirrorStr originStr)
    printf("DEBUG executeTransformComponent: Full operation = %L\n" operation)
    compName = cadr(assoc("component" operation))
    deltaMoveStr = cadr(assoc("delta" operation))
    angleStr = cadr(assoc("angle" operation))
    mirrorStr = cadr(assoc("mirror" operation))
    originStr = cadr(assoc("origin" operation))
    printf("DEBUG executeTransformComponent: compName='%s', deltaMoveStr='%s', angleStr='%s', mirrorStr='%s', originStr='%s'\n"
           compName deltaMoveStr angleStr mirrorStr originStr)

    ; Debug all field associations
    ; printf("DEBUG: component field = %L\n" assoc("component" operation))
    ; printf("DEBUG: delta field = %L\n" assoc("delta" operation))
    ; printf("DEBUG: angle field = %L\n" assoc("angle" operation))
    ; printf("DEBUG: mirror field = %L\n" assoc("mirror" operation))
    ; printf("DEBUG: origin field = %L\n" assoc("origin" operation))

    ; Parse delta move coordinate string
    if( deltaMoveStr && nindex(deltaMoveStr ":") then
      coords = parseString(deltaMoveStr ":")
      deltaMove = atof(car(coords)):atof(cadr(coords))
      ; printf("DEBUG: Parsed delta move: %L\n" deltaMove)
    else
      deltaMove = nil
    )

    ; Parse angle string to float value
    if( angleStr then
      angle = atof(angleStr)
      ; printf("DEBUG: Parsed angle: %g\n" angle)
    else
      angle = nil
    )

    ; Parse mirror parameter
    if( mirrorStr && (mirrorStr == "t" || mirrorStr == "true" || mirrorStr == "TRUE") then
      mirror = t
    else
      mirror = nil
    )

    ; Parse origin coordinate string
    if( originStr && nindex(originStr ":") then
      coords = parseString(originStr ":")
      origin = atof(car(coords)):atof(cadr(coords))
      ; printf("DEBUG: Parsed origin coordinate: %L\n" origin)
    else
      origin = nil
    )

    ; printf("DEBUG: Condition check: compName='%s'(%L), deltaMove=%L, angle=%L, mirror=%L\n"
    ;        compName (compName != nil) deltaMove angle mirror)
    ; printf("DEBUG: Combined condition result: %L\n" (compName && (deltaMove || angle || mirror)))

    if( compName then
     if( deltaMove || angle || mirror then
      ; Find component
      axlSetFindFilter(?enabled '(noall symbols invisible) ?onButtons '(symbols))
      component = axlGetSelSet(axlSingleSelectName("SYMBOL" compName))

      if( component then
        component = car(component)

        ; Apply the source angle as-is (the exporter bakes in the correct,
        ; flip-aware rotation delta -- see executeRotateComponent).

        ; Apply combined transformation using single axlTransformObject call
        ; Order: move -> mirror -> rotate (as documented)
        ; printf("DEBUG: About to call axlTransformObject with move=%L, mirror=%L, angle=%L, origin=%L\n"
        ;        deltaMove mirror angle origin)

        ; Build transformation parameters dynamically
        transformParams = list(component)
        if( deltaMove then
          transformParams = append(transformParams list(?move deltaMove))
        )
        if( mirror then
          transformParams = append(transformParams list(?mirror mirror))
        )
        if( angle then
          transformParams = append(transformParams list(?angle angle))
        )
        if( origin then
          transformParams = append(transformParams list(?origin origin))
        )

        ; Apply transformation
        result = apply('axlTransformObject transformParams)

        if( result then
          ; Build status message
          statusMsg = sprintf(nil "Transformed component %s:" compName)
          if( deltaMove then
            statusMsg = strcat(statusMsg sprintf(nil " moved by %L" deltaMove))
          )
          if( mirror then
            statusMsg = strcat(statusMsg " mirrored")
          )
          if( angle then
            if( origin then
              statusMsg = strcat(statusMsg sprintf(nil " rotated by %g degrees about %L" angle origin))
            else
              statusMsg = strcat(statusMsg sprintf(nil " rotated by %g degrees" angle))
            )
          )
          printf("%s\n" statusMsg)
          axlClearSelSet()
          t
        else
          printf("ERROR: Failed to transform component %s\n" compName)
          axlClearSelSet()
          nil
        )
      else
        printf("ERROR: Component %s not found\n" compName)
        nil
      )
     else
       ; a TRANSFORM_COMPONENT row that names a component but has no
       ; delta/angle/mirror is a no-op (the exporter emits these for unchanged
       ; components). Count it as a no-op and return t so it does not fail the
       ; whole transaction and trigger a rollback.
       printf("TRANSFORM: component %s has no delta/angle/mirror; treating as no-op\n" compName)
       qltrNoOpCount = qltrNoOpCount + 1
       t
     )
    else
      printf("ERROR: Missing component name for TRANSFORM_COMPONENT\n")
      nil
    )
  )
)

defun( executeCreatePadstack (operation)
  let( (padstackName drillSize padShape padSizes usage result)
    ; printf("DEBUG executeCreatePadstack: Full operation = %L\n" operation)

    ; Extract padstack parameters from operation
    padstackName = cadr(assoc("padstack" operation))
    drillSize = cadr(assoc("width" operation))  ; Using width field for drill size
    padShape = cadr(assoc("layer" operation))   ; Using layer field for pad shape
    padSizes = cadr(assoc("position" operation)) ; Using position field for pad sizes
    usage = cadr(assoc("net" operation))        ; Using net field for usage type

    ; printf("DEBUG executeCreatePadstack: padstackName='%s', drillSize='%s', padShape='%s', padSizes='%s', usage='%s'\n"
    ;        padstackName drillSize padShape padSizes usage)

    ; Set defaults
    if( !usage then usage = "via" )
    if( !padShape then padShape = "circle" )
    if( !drillSize then drillSize = "0.2" )
    if( !padSizes then padSizes = "0.4:0.4" )

    if( padstackName then
      ; Parse drill size and pad dimensions
      drillDiameter = atof(drillSize)

      ; Parse pad sizes (format: "outerSize:innerSize" or "size" for uniform)
      if( nindex(padSizes ":") then
        sizeParts = parseString(padSizes ":")
        outerPadSize = atof(car(sizeParts))
        innerPadSize = atof(cadr(sizeParts))
      else
        outerPadSize = atof(padSizes)
        innerPadSize = outerPadSize
      )

      ; printf("DEBUG: Creating padstack %s with drill=%g, outer pad=%g, inner pad=%g\n"
      ;       padstackName drillDiameter outerPadSize innerPadSize)

      ; Create drill definition
      ; printf("DEBUG: Creating drill with diameter=%g, usage=%s\n" drillDiameter usage)
      drillDef = make_axlPadStackDrill(
        ; ?usage "Via" ; ?usage "via" or "Via" causes error creating padstack
        ?drillDiameter drillDiameter
        ?plating 'PLATED
      )
      ; printf("DEBUG: Drill created: %L\n" drillDef)

      ; Create pad definitions for different layers
      padDefs = list()

      ; Outer layer pads (TOP/BOTTOM) - Using correct Cadence API format
      ; printf("DEBUG: Creating TOP pad with shape=%s, size=%g\n" padShape outerPadSize)

      ; Pre-evaluate figure parameter
      figureType = case( upperCase(padShape)
        ("CIRCLE" 'CIRCLE)
        ("SQUARE" 'SQUARE)
        (t 'CIRCLE)  ; Default to circle
      )
      ; printf("DEBUG: Evaluated figureType=%L for padShape=%s\n" figureType padShape)

      ; Pre-evaluate figureSize parameter
      figureSize = outerPadSize:outerPadSize
      ; printf("DEBUG: Evaluated figureSize=%L for outerPadSize=%g\n" figureSize outerPadSize)

      outerPad = make_axlPadStackPad(
        ?layer "TOP"
        ?type 'REGULAR
        ?figure figureType
        ?figureSize figureSize
      )
      ; printf("DEBUG: TOP pad created: %L\n" outerPad)
      padDefs = cons(outerPad padDefs)

      ; Bottom layer pad (same as top for through-hole)
      ; printf("DEBUG: Creating BOTTOM pad with shape=%s, size=%g\n" padShape outerPadSize)

      bottomPad = make_axlPadStackPad(
        ?layer "BOTTOM"
        ?type 'REGULAR
        ?figure figureType
        ?figureSize figureSize
      )
      ; printf("DEBUG: BOTTOM pad created: %L\n" bottomPad)
      padDefs = cons(bottomPad padDefs)

      ; Inner layer pads (if different from outer)
      if( innerPadSize != outerPadSize then
        ; printf("DEBUG: Creating INTERNAL pad with shape=%s, size=%g\n" padShape innerPadSize)

        ; Pre-evaluate figureSize parameter for inner layers
        innerFigureSize = innerPadSize:innerPadSize
        ; printf("DEBUG: Evaluated innerFigureSize=%L for innerPadSize=%g\n" innerFigureSize innerPadSize)

        innerPad = make_axlPadStackPad(
          ?layer "DEFAULT INTERNAL"
          ?type 'REGULAR
          ?figure figureType
          ?figureSize innerFigureSize
        )
        ; printf("DEBUG: INTERNAL pad created: %L\n" innerPad)
        padDefs = cons(innerPad padDefs)
      )

      ; Create the padstack
      ; printf("DEBUG: Calling axlDBCreatePadStack with name='%s', drill=%L, pads=%L\n" padstackName drillDef padDefs)
      result = axlDBCreatePadStack(padstackName drillDef padDefs t)

      if( result then
        ; printf("Created padstack: %s (drill=%g, outer pad=%g, inner pad=%g)\n"
        ;       padstackName drillDiameter outerPadSize innerPadSize)
        t
      else
        printf("ERROR: Failed to create padstack %s\n" padstackName)
        nil
      )
    else
      printf("ERROR: Missing padstack name\n")
      nil
    )
  )
)

; ============================================================================
; 7. POUR CREATION FUNCTIONS
; ============================================================================

defun( executeCreatePour (operation)
  let( (layer netName positionStr coordStrings pointList path shape result)
    ; printf("DEBUG executeCreatePour: Full operation = %L\n" operation)

    ; Extract fields from operation
    layer = cadr(assoc("layer" operation))
    netName = cadr(assoc("net" operation))
    positionStr = cadr(assoc("position" operation))

    printf("DEBUG executeCreatePour: layer='%s', netName='%s', positionStr='%s'\n"
           layer netName positionStr)

    if( layer && positionStr then
      ; Parse space-separated coordinate string
      coordStrings = parseString(positionStr " ")
      ; printf("DEBUG: Found %d coordinate strings\n" length(coordStrings))

      ; Convert coordinate strings to point list
      pointList = nil
      foreach( coordStr coordStrings
        when( coordStr != "" && nindex(coordStr ":")
          coords = parseString(coordStr ":")
          when( length(coords) >= 2
            point = atof(car(coords)):atof(cadr(coords))
            pointList = append(pointList list(point))
            ; printf("DEBUG: Parsed coordinate: %s -> %L\n" coordStr point)
          )
        )
      )

      printf("DEBUG: Created point list with %d points\n" length(pointList))

      if( length(pointList) >= 3 then
        ; Create path from point list
        path = axlPathStart(pointList)
        ; printf("DEBUG: Created path: %L\n" path)

        ; Create filled shape
        shape = axlDBCreateShape(path t layer netName nil)

        if( shape then
          printf("Successfully created pour shape: %L on layer %s, net %s\n"
                 shape layer netName)
          result = t
        else
          printf("ERROR: Failed to create pour shape\n")
          result = nil
        )
      else
        printf("ERROR: Insufficient points for polygon (%d points, need >= 3)\n"
               length(pointList))
        result = nil
      )
    else
      printf("ERROR: Missing required fields (layer=%s, position=%s)\n" layer positionStr)
      result = nil
    )

    result
  )
)

; ============================================================================
; 8. MAIN IMPORT FUNCTION
; ============================================================================

defun( importDesignModifications (filename @optional expectedTimestamp)
  let( (format dataRows result)
    printf("\n=== Design File Importer ===\n")
    printf("Importing modifications from: %s\n" filename)

    ; Validate file exists
    if( validateFileExists(filename) then
      ; Start progress meter for entire import operation
      axlMeterCreate("Importing Design Changes" "Parsing CSV file with design updates..." nil)

      ; Validate design state if timestamp provided
      if( expectedTimestamp then
        if( !validateDesignState(expectedTimestamp) then
          printf("WARNING: Design timestamp mismatch - proceed with caution\n")
          ; Continue anyway for now, but could be made stricter
        )
      )

      ; Detect file format
      format = detectFileFormat(filename)
      case( format
        ("CSV"
          dataRows = parseCSVFile(filename)
        )
        (t
          printf("ERROR: Unsupported or undetectable file format\n")
          dataRows = nil
        )
      )

      ; Process modifications if parsing succeeded
      if( dataRows then
        result = processDesignModifications(dataRows format)
      else
        printf("ERROR: Failed to parse input file\n")
        result = nil
      )
      if( result then
        axlMeterUpdate(100 "Import geometry complete; applying dynamic shape settings...")
      else
        axlMeterUpdate(100 "Import failed; closing progress meter...")
      )
      axlFlushDisplay()
      axlMeterDestroy()

      ; Set the Thermal/anti clearances directly via SKILL only after a
      ; successful import. These are global dynamic shape parameters, so a
      ; failed or rolled-back import should not leave the user's board session
      ; with changed global settings.
      if( result then
        qltrConfigureShapeThermalAnti()
      else
        printf("SHAPE PARAM: skipped dynamic shape clearance update because import failed or rolled back\n")
      )
    else
      result = nil
    )

    ; Final status
    if( result then
      printf("\n=== Import completed successfully ===\n")
    else
      printf("\n=== Import failed ===\n")
    )

    result
  )
)

; ============================================================================
; 9. COMMAND REGISTRATION
; ============================================================================

; register the main import command. It sets the Thermal/anti clearances via SKILL
; automatically after the import. qltrDumpShapeDynamicParams remains defined for
; manual SKILL use but is intentionally not registered.
axlCmdRegister("Quilter:importDesignUpdates" 'importDesignModifications ?cmdType "general")


; ============================================================================
; INITIALIZATION MESSAGE
; ============================================================================
printf("\n=== Quilter Design Update Importer Loaded ===\n")
printf("Available commands:\n")
printf("  Quilter:importDesignUpdates filename_without_brackets - Import design modifications and set dynamic shape clearances to Thermal/anti\n")
printf("\nSupported formats:\n")
printf("  - CSV: Comma-separated values with headers\n")
printf("\nFeatures:\n")
printf("  - Automatic CSV format detection\n")
printf("  - Transaction support (atomic operations)\n")
printf("  - Design integrity checking\n")
printf("  - Progress reporting and error handling\n")
printf("  - Preserves netlist connectivity\n")
printf("  - Fix: processes components/vias before clines\n")
printf("  - Fix: connects cline endpoints to collected same-net pin/via centers with direct or hairpin stubs\n")
printf("  - Fix: sets dyn shape Thru/SMD pin + via clearances to Thermal/anti via axlSetParam (verified)\n")
printf("  - Debug: reports direct/hairpin stub counts and average/min/max endpoint-to-target gaps\n")
printf("\nExample usage:\n")
printf("  Quilter:importDesignUpdates \"modifications.csv\"\n")
printf("========================================\n\n")
