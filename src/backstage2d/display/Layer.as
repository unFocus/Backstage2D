package backstage2d.display 
{
	import flash.geom.Point;
	
	/**
	 * ...
	 * @author Kevin Newman
	 */
	public class Layer 
	{
		public var actors:Vector.<Actor> = new <Actor>[];
		
		public var x:Number = 0;
		public var y:Number = 0;
		public var z:Number = 0;
		
		public var registration:Point;
		
		public var rotation:Number = 0;
		
		public function add( child:Actor ):void
		{
			actors.push( child );
		}
		
	}
}
